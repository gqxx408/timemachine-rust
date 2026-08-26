use serde::Deserialize;
use std::path::Path;
use crate::model::HashAlgo;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub backup: BackupConfig,
    pub network: NetworkConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Deserialize)]
pub struct BackupConfig {
    pub hash_algo: String,
    pub scan_threads: usize,
    pub copy_concurrency: usize,
    pub skip_hidden: bool,
}

#[derive(Debug, Deserialize)]
pub struct NetworkConfig {
    pub listen_addr: String,
    pub chunk_size: usize,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("无法读取配置文件 {}: {}", path, e))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("配置文件解析失败: {}", e))?;
        Ok(config)
    }

    pub fn hash_algo(&self) -> HashAlgo {
        HashAlgo::from_str(&self.backup.hash_algo)
    }

    pub fn init_logging(&self) {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&self.logging.level));

        match self.logging.format.as_str() {
            "json" => {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .json()
                    .init();
            }
            _ => {
                tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .init();
            }
        }
    }

    pub fn ensure_dirs(&self, base: &Path) {
        if self.database.url.starts_with("sqlite://") {
            let db_path = self.database.url
                .strip_prefix("sqlite://")
                .unwrap_or("data/timemachine.db");
            let db_path = db_path.split('?').next().unwrap_or(db_path);
            if let Some(parent) = Path::new(db_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let _ = std::fs::create_dir_all(base.join("logs"));
    }
}
