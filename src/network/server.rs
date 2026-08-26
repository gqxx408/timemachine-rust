use std::path::Path;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use crate::network::{cmd, FileInfoDto, Request, Response, read_frame, write_frame};

pub async fn run_server(
    listen_addr: &str,
    root_path: &str,
    skip_hidden: bool,
    chunk_size: usize,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen_addr).await?;
    tracing::info!(addr = listen_addr, root = root_path, "服务端启动, 等待连接...");

    loop {
        let (mut socket, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "accept 失败");
                continue;
            }
        };
        let root = root_path.to_string();
        tokio::spawn(async move {
            tracing::info!(peer = %peer, "新连接");
            if let Err(e) = handle_connection(&mut socket, &root, skip_hidden, chunk_size).await {
                tracing::warn!(peer = %peer, error = %e, "连接结束");
            }
        });
    }
}

async fn handle_connection(
    socket: &mut tokio::net::TcpStream,
    root: &str,
    skip_hidden: bool,
    chunk_size: usize,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let (mut reader, mut writer) = socket.split();

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };

        let req: Request = serde_json::from_slice(&frame)?;
        tracing::debug!(cmd = req.cmd, "收到请求");

        match req.cmd.as_str() {
            cmd::LIST => {
                let files = scan_directory(root, skip_hidden)?;
                let dtos: Vec<FileInfoDto> = files.iter().map(FileInfoDto::from).collect();
                let json = serde_json::to_vec(&dtos)?;
                write_frame(&mut writer, &json).await?;
            }
            cmd::HASH => {
                let path = req.path.as_deref().unwrap_or("");
                let algo = req.algo.as_deref().unwrap_or("blake3");
                let full_path = resolve_path(root, path);
                match compute_hash(&full_path, algo) {
                    Ok(hash) => {
                        let resp = Response { ok: true, error: None, hash: Some(hash), size: None };
                        let json = serde_json::to_vec(&resp)?;
                        write_frame(&mut writer, &json).await?;
                    }
                    Err(e) => {
                        let resp = Response { ok: false, error: Some(e), hash: None, size: None };
                        let json = serde_json::to_vec(&resp)?;
                        write_frame(&mut writer, &json).await?;
                    }
                }
            }
            cmd::READ => {
                let path = req.path.as_deref().unwrap_or("");
                let full_path = resolve_path(root, path);
                match std::fs::File::open(&full_path) {
                    Ok(file) => {
                        let size = file.metadata()?.len();
                        let header = Response { ok: true, error: None, hash: None, size: Some(size) };
                        let json = serde_json::to_vec(&header)?;
                        write_frame(&mut writer, &json).await?;

                        let mut reader_file = tokio::fs::File::from_std(file);
                        let mut buf = vec![0u8; chunk_size];
                        loop {
                            let n = reader_file.read(&mut buf).await?;
                            if n == 0 {
                                break;
                            }
                            write_frame(&mut writer, &buf[..n]).await?;
                        }
                        write_frame(&mut writer, &[]).await?;
                    }
                    Err(e) => {
                        let resp = Response { ok: false, error: Some(e.to_string()), hash: None, size: None };
                        let json = serde_json::to_vec(&resp)?;
                        write_frame(&mut writer, &json).await?;
                    }
                }
            }
            cmd::QUIT => break,
            other => {
                let resp = Response { ok: false, error: Some(format!("未知命令: {}", other)), hash: None, size: None };
                let json = serde_json::to_vec(&resp)?;
                write_frame(&mut writer, &json).await?;
            }
        }
        writer.flush().await?;
    }
    Ok(())
}

fn resolve_path(root: &str, rel_path: &str) -> std::path::PathBuf {
    let root = Path::new(root);
    if Path::new(rel_path).is_absolute() {
        root.join(rel_path.trim_start_matches('/'))
    } else {
        root.join(rel_path)
    }
}

fn scan_directory(root: &str, skip_hidden: bool) -> anyhow::Result<Vec<crate::model::ScannedFile>> {
    use jwalk::WalkDir;
    let mut files = Vec::new();
    let walker = WalkDir::new(root).follow_links(false);

    for entry in walker {
        let entry = entry?;
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

        let meta = std::fs::metadata(&path)?;
        let mtime = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        files.push(crate::model::ScannedFile {
            path,
            size: meta.len(),
            mtime,
        });
    }
    Ok(files)
}

fn compute_hash(path: &Path, algo: &str) -> std::result::Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    match algo {
        "md5" => {
            use md5::{Digest, Md5};
            let mut hasher = Md5::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = std::io::Read::read(&mut file, &mut buf).map_err(|e| e.to_string())?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        _ => {
            let mut hasher = blake3::Hasher::new();
            let mut reader = std::io::BufReader::new(file);
            std::io::copy(&mut reader, &mut hasher).map_err(|e| e.to_string())?;
            Ok(hasher.finalize().to_hex().to_string())
        }
    }
}
