use std::sync::Arc;
use indicatif::{ProgressBar, ProgressStyle};
use crate::balancer::DiskBalancer;
use crate::copier;
use crate::differ;
use crate::error::{BackupError, Result};
use crate::model::*;
use crate::network::{FileSource, LocalSource, RemoteSource};
use crate::progress::ProgressTracker;
use crate::store::MetadataStore;

pub struct BackupEngine {
    store: Arc<dyn MetadataStore>,
    hash_algo: HashAlgo,
    #[allow(dead_code)]
    copy_concurrency: usize,
    skip_hidden: bool,
    progress: Option<Arc<ProgressTracker>>,
}

impl BackupEngine {
    pub fn new(
        store: Arc<dyn MetadataStore>,
        hash_algo: HashAlgo,
        copy_concurrency: usize,
        skip_hidden: bool,
    ) -> Self {
        Self { store, hash_algo, copy_concurrency, skip_hidden, progress: None }
    }

    pub fn with_progress(mut self, tracker: Arc<ProgressTracker>) -> Self {
        self.progress = Some(tracker);
        self
    }

    fn on_progress<F>(&self, f: F)
    where
        F: FnOnce(&ProgressTracker),
    {
        if let Some(p) = &self.progress {
            f(p);
        }
    }

    pub async fn run(&self) -> Result<()> {
        let roots = self.store.load_backup_roots().await?;
        let targets = self.store.load_backup_targets().await?;

        if targets.is_empty() {
            return Err(BackupError::NoAvailableTarget);
        }

        for target in &targets {
            let _ = copier::ensure_target_dir(target);
        }

        let balancer = DiskBalancer::new(targets);
        let session_id = self.store.begin_session().await?;
        tracing::info!(session_id, "备份会话开始");
        self.on_progress(|p| {
            p.start();
            p.log("备份会话开始".to_string());
        });

        let mut total_files = 0u64;
        let mut total_bytes = 0u64;
        let mut has_error = false;

        for root in &roots {
            self.on_progress(|p| {
                p.set_current_root(&root.root_path);
            });
            match self.backup_root(root, &balancer, session_id).await {
                Ok((files, bytes)) => {
                    total_files += files;
                    total_bytes += bytes;
                }
                Err(e) => {
                    tracing::error!(root_id = root.id, error = %e, "备份根目录失败");
                    has_error = true;
                    self.on_progress(|p| {
                        p.log(format!("备份源失败: {}", e));
                    });
                }
            }
        }

        let status = if has_error { "failed" } else { "completed" };
        self.store
            .finish_session(session_id, total_files, total_bytes, status)
            .await?;
        tracing::info!(
            session_id, total_files, total_bytes, status, "备份会话结束"
        );
        self.on_progress(|p| {
            p.log(format!(
                "备份完成: {} 文件, {:.2} MB",
                total_files,
                total_bytes as f64 / 1_048_576.0
            ));
        });

        self.print_balancer_stats(&balancer);
        Ok(())
    }

    async fn backup_root(
        &self,
        root: &BackupRoot,
        balancer: &DiskBalancer,
        session_id: i64,
    ) -> Result<(u64, u64)> {
        tracing::info!(
            root_id = root.id,
            path = root.root_path,
            source_type = root.source_type,
            "开始备份"
        );

        let source: Box<dyn FileSource> = match SourceType::from_str(&root.source_type) {
            SourceType::Local => {
                let p = std::path::Path::new(&root.root_path);
                if !p.exists() {
                    return Err(BackupError::FileNotFound(root.root_path.clone()));
                }
                Box::new(LocalSource::new(&root.root_path, self.skip_hidden))
            }
            SourceType::Remote => {
                let addr = &root.root_path;
                tracing::info!(addr, "连接远程备份源...");
                Box::new(RemoteSource::connect(addr).await?)
            }
        };

        let files = source.list_files().await?;
        tracing::info!(root_id = root.id, count = files.len(), "扫描完成");
        self.on_progress(|p| {
            p.set_total_files(files.len() as u64);
        });

        let tracked_map = self.store.load_tracked_files_map(root.id).await?;

        let mut file_ids: Vec<(ScannedFile, i64)> = Vec::with_capacity(files.len());
        for file in &files {
            let path_str = file.path.to_string_lossy().to_string();
            let id = match tracked_map.get(&path_str) {
                Some(&id) => id,
                None => self.store.insert_tracked_file(root.id, &path_str).await?,
            };
            file_ids.push((file.clone(), id));
        }

        let ids: Vec<i64> = file_ids.iter().map(|(_, id)| *id).collect();
        let latest_versions = self.store.load_latest_versions(&ids).await?;

        let pb = ProgressBar::new(file_ids.len() as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{elapsed} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}"
            )
            .unwrap()
        );

        let mut copy_count = 0u64;
        let mut copy_bytes = 0u64;
        let algo_str = self.hash_algo.as_str();

        for (file, file_id) in &file_ids {
            pb.inc(1);
            let path_str = file.path.to_string_lossy().to_string();
            self.on_progress(|p| p.set_current_file(&path_str));

            let latest = latest_versions.get(file_id);
            let diff = differ::check_changed(file, latest);
            if !diff.needs_backup {
                self.on_progress(|p| p.file_done(0));
                continue;
            }

            let hash = match source.hash_file(&path_str, algo_str).await {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(path = %path_str, error = %e, "哈希计算失败, 跳过");
                    self.on_progress(|p| p.file_done(0));
                    continue;
                }
            };

            if let Some(latest) = latest {
                if latest.content_hash == hash {
                    self.on_progress(|p| p.file_done(0));
                    continue;
                }
            }

            let (target, stats) = match balancer.select_target(file.size) {
                Some(t) => t,
                None => {
                    tracing::error!(size = file.size, "无可用目标盘, 跳过");
                    self.on_progress(|p| p.file_done(0));
                    continue;
                }
            };
            stats.acquire();

            let rel_path = copier::build_rel_path(target, &hash);
            let is_new = match self
                .store
                .upsert_content_block(
                    &hash,
                    self.hash_algo,
                    file.size as i64,
                    &rel_path,
                    target.id,
                )
                .await
            {
                Ok(n) => n,
                Err(e) => {
                    stats.release(0);
                    tracing::error!(error = %e, "数据库写入失败");
                    self.on_progress(|p| p.file_done(0));
                    continue;
                }
            };

            let mut bytes_written: u64 = 0;
            if is_new {
                let dest_path = copier::build_dest_path(target, &rel_path);
                match source.copy_file_to(&path_str, &dest_path.to_string_lossy()).await {
                    Ok(bytes) => {
                        copy_bytes += bytes;
                        bytes_written = bytes;
                        tracing::debug!(
                            src = %path_str,
                            dest = %dest_path.display(),
                            bytes,
                            "已拷贝"
                        );
                    }
                    Err(e) => {
                        stats.release(0);
                        tracing::error!(path = %path_str, error = %e, "拷贝失败");
                        self.on_progress(|p| p.file_done(0));
                        continue;
                    }
                }
            }

            if let Err(e) = self
                .store
                .insert_file_version(*file_id, session_id, &hash, file.mtime, file.size as i64)
                .await
            {
                tracing::error!(error = %e, "版本记录失败");
            }

            stats.release(file.size);
            self.on_progress(|p| p.file_done(bytes_written));
            copy_count += 1;
        }

        pb.finish_with_message(format!(
            "根目录 {} 完成: {} 文件, {:.2} MB",
            root.id,
            copy_count,
            copy_bytes as f64 / 1_048_576.0
        ));
        Ok((copy_count, copy_bytes))
    }

    fn print_balancer_stats(&self, balancer: &DiskBalancer) {
        let snapshots = balancer.snapshot();
        tracing::info!("=== 磁盘负载统计 ===");
        for s in &snapshots {
            tracing::info!(
                "盘 #{}: {} | 活跃写入={} | 本会话写入={:.2}MB | 剩余={:.2}GB",
                s.target_id,
                s.target_path,
                s.active_writes,
                s.session_bytes as f64 / 1_048_576.0,
                s.free_space as f64 / 1_073_741_824.0
            );
        }
    }
}
