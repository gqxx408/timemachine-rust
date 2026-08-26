use clap::Parser;
use std::sync::Arc;
use tm_rust::cli::{Cli, Commands};
use tm_rust::config::Config;
use tm_rust::engine::BackupEngine;
use tm_rust::network::{FileSource, RemoteSource};
use tm_rust::network::server;
use tm_rust::store::sqlite::SqliteStore;
use tm_rust::store::MetadataStore;
use tm_rust::verifier::Verifier;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Server { root, addr } => {
            let config = load_config(&cli.config)?;
            let listen = addr.clone().unwrap_or_else(|| config.network.listen_addr.clone());
            let skip = config.backup.skip_hidden;
            let chunk = config.network.chunk_size;
            tracing::info!(root, addr = %listen, "服务端启动中...");
            server::run_server(&listen, root, skip, chunk).await?;
            return Ok(());
        }
        Commands::Ping { addr } => {
            println!("连接 {} ...", addr);
            let source = RemoteSource::connect(addr).await
                .map_err(|e| anyhow::anyhow!("连接失败: {}", e))?;
            let files = source.list_files().await
                .map_err(|e| anyhow::anyhow!("获取文件列表失败: {}", e))?;
            println!("连接成功! 远程文件数: {}", files.len());
            for f in files.iter().take(10) {
                println!("  {} ({} bytes, mtime={})", f.path.display(), f.size, f.mtime);
            }
            if files.len() > 10 {
                println!("  ... (共 {} 个文件)", files.len());
            }
            let _ = source.quit().await;
            return Ok(());
        }
        Commands::Gui => {
            let config = load_config(&cli.config)?;
            tm_rust::gui::run_gui(config)?;
            return Ok(());
        }
        _ => {}
    }

    let config = load_config(&cli.config)?;
    let store = create_store(&config).await?;

    match &cli.command {
        Commands::Init => {
            println!("数据库已初始化: {}", config.database.url);
        }

        Commands::Backup => {
            let engine = BackupEngine::new(
                store,
                config.hash_algo(),
                config.backup.copy_concurrency,
                config.backup.skip_hidden,
            );
            engine.run().await?;
        }

        Commands::Check { with_hash } => {
            let verifier = Verifier::new(store, config.hash_algo());
            verifier.check_data(*with_hash).await?;
        }

        Commands::Gc { root_id } => {
            let verifier = Verifier::new(store, config.hash_algo());
            verifier.delete_by_root(*root_id).await?;
        }

        Commands::AddRoot { path, source_type, label } => {
            let id = store.add_backup_root(path, source_type, label.as_deref()).await?;
            println!("已添加备份源 #{} [{}] {}", id, source_type, path);
        }

        Commands::AddTarget { path, subdir, quota } => {
            let id = store.add_backup_target(path, subdir, *quota).await?;
            println!("已添加目标盘 #{} {} (子目录: {})", id, path, subdir);
        }

        Commands::List => {
            let roots = store.load_backup_roots().await?;
            let targets = store.load_backup_targets().await?;

            println!("\n备份源 ({}):", roots.len());
            for r in &roots {
                println!("  #{} [{}] {} {}",
                    r.id, r.source_type, r.root_path,
                    r.label.as_deref().unwrap_or("")
                );
            }

            println!("\n目标盘 ({}):", targets.len());
            for t in &targets {
                let free = get_free_space(&t.target_path);
                println!("  #{} {} (子目录: {}, 剩余: {:.1} GB)",
                    t.id, t.target_path, t.subdir_name,
                    free as f64 / 1_073_741_824.0
                );
            }
            println!();
        }

        Commands::Server { .. } | Commands::Ping { .. } | Commands::Gui => unreachable!(),
    }

    Ok(())
}

fn load_config(path: &str) -> anyhow::Result<Config> {
    let config = Config::load(path)?;
    config.init_logging();
    config.ensure_dirs(std::path::Path::new("."));
    Ok(config)
}

async fn create_store(config: &Config) -> anyhow::Result<Arc<dyn MetadataStore>> {
    let store = SqliteStore::new(&config.database.url).await?;
    store.apply_migrations().await?;
    Ok(Arc::new(store))
}

fn get_free_space(path: &str) -> u64 {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let drive = if path.len() >= 2 && path.as_bytes()[1] == b':' {
            format!("{}\\", &path[..2])
        } else {
            path.to_string()
        };
        let wide: Vec<u16> = OsStr::new(&drive)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut avail: u64 = 0;
        let mut total: u64 = 0;
        let mut free: u64 = 0;
        extern "system" {
            fn GetDiskFreeSpaceExW(
                dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64,
            ) -> i32;
        }
        let ret = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
        if ret != 0 { avail } else { 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        0
    }
}
