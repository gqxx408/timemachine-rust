use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use crate::error::{BackupError, Result};
use crate::model::{HashAlgo, ScannedFile};
use super::{cmd, FileInfoDto, FileSource, Request, Response, read_frame, write_frame};

// ========== 本地文件源 ==========

pub struct LocalSource {
    root_path: String,
    skip_hidden: bool,
}

impl LocalSource {
    pub fn new(root_path: &str, skip_hidden: bool) -> Self {
        Self {
            root_path: root_path.to_string(),
            skip_hidden,
        }
    }
}

#[async_trait]
impl FileSource for LocalSource {
    async fn list_files(&self) -> Result<Vec<ScannedFile>> {
        let root = self.root_path.clone();
        let skip = self.skip_hidden;
        let files = tokio::task::spawn_blocking(move || scan_local(&root, skip))
            .await
            .map_err(|e| BackupError::Io(e.into()))??;
        Ok(files)
    }

    async fn hash_file(&self, path: &str, algo: &str) -> Result<String> {
        let path = path.to_string();
        let algo = HashAlgo::from_str(algo);
        tokio::task::spawn_blocking(move || hash_local(&path, algo))
            .await
            .map_err(|e| BackupError::Io(e.into()))?
    }

    async fn copy_file_to(&self, path: &str, dest_path: &str) -> Result<u64> {
        let dest = dest_path.to_string();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
            if let Some(parent) = Path::new(&dest).parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &dest)
        })
        .await
        .map_err(|e| BackupError::Io(e.into()))?
        .map_err(BackupError::Io)
    }
}

// ========== 远程文件源 ==========

pub struct RemoteSource {
    stream: Arc<Mutex<TcpStream>>,
}

impl RemoteSource {
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await.map_err(|e| {
            BackupError::Network(format!("连接 {} 失败: {}", addr, e))
        })?;
        tracing::info!(addr = addr, "已连接到远程服务端");
        Ok(Self {
            stream: Arc::new(Mutex::new(stream)),
        })
    }

    pub async fn quit(&self) -> Result<()> {
        let req = Request {
            cmd: cmd::QUIT.to_string(),
            path: None,
            algo: None,
        };
        let json = serde_json::to_vec(&req).unwrap_or_default();
        let mut stream = self.stream.lock().await;
        write_frame(&mut *stream, &json).await.ok();
        Ok(())
    }
}

#[async_trait]
impl FileSource for RemoteSource {
    async fn list_files(&self) -> Result<Vec<ScannedFile>> {
        let req = Request {
            cmd: cmd::LIST.to_string(),
            path: None,
            algo: None,
        };
        let json = serde_json::to_vec(&req).map_err(|e| BackupError::Network(e.to_string()))?;

        let mut stream = self.stream.lock().await;
        write_frame(&mut *stream, &json).await?;
        let frame = read_frame(&mut *stream).await?;

        let dtos: Vec<FileInfoDto> =
            serde_json::from_slice(&frame).map_err(|e| BackupError::Network(e.to_string()))?;

        Ok(dtos.into_iter().map(ScannedFile::from).collect())
    }

    async fn hash_file(&self, path: &str, algo: &str) -> Result<String> {
        let req = Request {
            cmd: cmd::HASH.to_string(),
            path: Some(path.to_string()),
            algo: Some(algo.to_string()),
        };
        let json = serde_json::to_vec(&req).map_err(|e| BackupError::Network(e.to_string()))?;

        let mut stream = self.stream.lock().await;
        write_frame(&mut *stream, &json).await?;
        let frame = read_frame(&mut *stream).await?;

        let resp: Response =
            serde_json::from_slice(&frame).map_err(|e| BackupError::Network(e.to_string()))?;

        if !resp.ok {
            return Err(BackupError::Network(
                resp.error.unwrap_or_else(|| "未知错误".to_string()),
            ));
        }
        resp.hash.ok_or_else(|| BackupError::Network("缺少 hash 字段".to_string()))
    }

    async fn copy_file_to(&self, path: &str, dest_path: &str) -> Result<u64> {
        let req = Request {
            cmd: cmd::READ.to_string(),
            path: Some(path.to_string()),
            algo: None,
        };
        let json = serde_json::to_vec(&req).map_err(|e| BackupError::Network(e.to_string()))?;

        let mut stream = self.stream.lock().await;
        write_frame(&mut *stream, &json).await?;

        let header_frame = read_frame(&mut *stream).await?;
        let header: Response =
            serde_json::from_slice(&header_frame).map_err(|e| BackupError::Network(e.to_string()))?;

        if !header.ok {
            return Err(BackupError::Network(
                header.error.unwrap_or_else(|| "读取失败".to_string()),
            ));
        }

        let total_size = header.size.unwrap_or(0);

        if let Some(parent) = Path::new(dest_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut dest_file = tokio::fs::File::create(dest_path).await?;
        let mut written = 0u64;

        loop {
            let data_frame = read_frame(&mut *stream).await?;
            if data_frame.is_empty() {
                break;
            }
            dest_file.write_all(&data_frame).await?;
            written += data_frame.len() as u64;
        }
        dest_file.flush().await?;

        if written != total_size {
            tracing::warn!(
                path = path,
                expected = total_size,
                actual = written,
                "文件大小不匹配"
            );
        }

        Ok(written)
    }
}

// ========== 本地辅助函数 ==========

fn scan_local(root: &str, skip_hidden: bool) -> std::io::Result<Vec<ScannedFile>> {
    use jwalk::WalkDir;
    let mut files = Vec::new();
    let walker = WalkDir::new(root).follow_links(false);

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();

        if skip_hidden {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }
        }

        if !entry.file_type().is_file() {
            continue;
        }

        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        files.push(ScannedFile {
            path,
            size: meta.len(),
            mtime,
        });
    }
    Ok(files)
}

fn hash_local(path: &str, algo: HashAlgo) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    match algo {
        HashAlgo::Md5 => {
            use md5::{Digest, Md5};
            let mut hasher = Md5::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = std::io::Read::read(&mut file, &mut buf)?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        HashAlgo::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            Ok(hasher.finalize().to_hex().to_string())
        }
    }
}
