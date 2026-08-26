use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use indicatif::{ProgressBar, ProgressStyle};
use crate::error::Result;
use crate::hasher;
use crate::model::HashAlgo;
use crate::store::MetadataStore;

pub struct Verifier {
    store: Arc<dyn MetadataStore>,
    #[allow(dead_code)]
    hash_algo: HashAlgo,
}

impl Verifier {
    pub fn new(store: Arc<dyn MetadataStore>, hash_algo: HashAlgo) -> Self {
        Self { store, hash_algo }
    }

    pub async fn check_data(&self, with_hash: bool) -> Result<()> {
        tracing::info!("开始数据完整性校验...");
        let blocks = self.store.load_all_blocks().await?;
        let targets = self.store.load_backup_targets().await?;

        let target_map: HashMap<i64, String> = targets
            .iter()
            .map(|t| (t.id, t.target_path.clone()))
            .collect();

        let pb = ProgressBar::new(blocks.len() as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{elapsed} [{bar:40.green}] {pos}/{len} ({percent}%) 损坏:{msg}"
            )
            .unwrap()
        );

        let mut broken = Vec::new();
        let mut broken_count = 0u64;

        for (hash, algo_str, size, rel_path, target_id) in &blocks {
            pb.inc(1);
            pb.set_message(format!("{}", broken_count));

            let target_root = match target_map.get(target_id) {
                Some(p) => p,
                None => {
                    tracing::warn!(target_id, "目标盘不存在, 标记为损坏");
                    broken.push(hash.clone());
                    broken_count += 1;
                    continue;
                }
            };

            let full_path = Path::new(target_root).join(rel_path);

            match std::fs::metadata(&full_path) {
                Ok(meta) => {
                    if meta.len() != *size as u64 {
                        tracing::warn!(
                            path = %full_path.display(),
                            expected = size,
                            actual = meta.len(),
                            "文件大小不匹配"
                        );
                        broken.push(hash.clone());
                        broken_count += 1;
                        continue;
                    }

                    if with_hash {
                        let algo = HashAlgo::from_str(algo_str);
                        let path_str = full_path.to_string_lossy().to_string();
                        let actual_hash = tokio::task::spawn_blocking(move || {
                            hasher::hash_file(Path::new(&path_str), algo)
                        })
                        .await
                        .map_err(|e| crate::error::BackupError::Io(e.into()))??;

                        if &actual_hash != hash {
                            tracing::warn!(
                                path = %full_path.display(),
                                expected = hash,
                                actual = %actual_hash,
                                "哈希不匹配"
                            );
                            broken.push(hash.clone());
                            broken_count += 1;
                        }
                    }
                }
                Err(_) => {
                    tracing::warn!(path = %full_path.display(), "文件不存在");
                    broken.push(hash.clone());
                    broken_count += 1;
                }
            }
        }

        pb.finish_with_message(format!("{} 个损坏", broken_count));

        if !broken.is_empty() {
            tracing::info!("开始清理 {} 个损坏块...", broken.len());
            for hash in &broken {
                if let Err(e) = self.store.decrement_block_ref(hash).await {
                    tracing::error!(hash, error = %e, "清理失败");
                }
            }
        }

        tracing::info!(
            total = blocks.len(),
            broken = broken.len(),
            "校验完成"
        );
        Ok(())
    }

    pub async fn delete_by_root(&self, root_id: i64) -> Result<()> {
        tracing::info!(root_id, "开始删除备份数据...");
        let versions = self.store.load_versions_by_root(root_id).await?;

        let pb = ProgressBar::new(versions.len() as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{elapsed} [{bar:40.red}] {pos}/{len} ({percent}%) {msg}"
            )
            .unwrap()
        );

        let mut deleted_files = 0u64;

        for (version_id, hash, _size, full_path, _tracked_file_id) in &versions {
            pb.inc(1);

            if let Err(e) = self.store.delete_version(*version_id).await {
                tracing::error!(version_id, error = %e, "删除版本记录失败");
                continue;
            }

            let block_deleted = self.store.decrement_block_ref(hash).await?;
            if block_deleted {
                if !full_path.is_empty() {
                    let _ = std::fs::remove_file(full_path);
                    deleted_files += 1;
                }
            }
        }

        pb.finish_with_message(format!("删除 {} 个文件", deleted_files));

        self.store.delete_tracked_files_by_root(root_id).await?;

        tracing::info!(
            root_id,
            versions = versions.len(),
            deleted_files,
            "删除完成"
        );
        Ok(())
    }
}
