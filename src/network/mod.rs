pub mod server;
pub mod client;

pub use client::{LocalSource, RemoteSource};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use crate::error::Result;
use crate::model::ScannedFile;

/// 文件源抽象: 本地文件和远程文件统一接口
/// 备份引擎通过此 trait 操作文件源, 无需关心来源
#[async_trait]
pub trait FileSource: Send + Sync {
    /// 列出所有文件 (路径、大小、修改时间)
    async fn list_files(&self) -> Result<Vec<ScannedFile>>;

    /// 计算文件哈希 (本地直接算, 远程请求服务端算)
    async fn hash_file(&self, path: &str, algo: &str) -> Result<String>;

    /// 将文件内容复制到目标路径, 返回写入字节数
    async fn copy_file_to(&self, path: &str, dest_path: &str) -> Result<u64>;
}

// ========== 协议帧工具 ==========

/// 写帧: [4 bytes BE length][payload]
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, data: &[u8]) -> std::io::Result<()> {
    let len = data.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

/// 读帧: 读取 4 字节长度, 再读取对应长度数据
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

// ========== 协议消息结构 ==========

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub cmd: String,
    pub path: Option<String>,
    pub algo: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfoDto {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

impl From<FileInfoDto> for ScannedFile {
    fn from(dto: FileInfoDto) -> Self {
        ScannedFile {
            path: dto.path.into(),
            size: dto.size,
            mtime: dto.mtime,
        }
    }
}

impl From<&ScannedFile> for FileInfoDto {
    fn from(f: &ScannedFile) -> Self {
        FileInfoDto {
            path: f.path.to_string_lossy().to_string(),
            size: f.size,
            mtime: f.mtime,
        }
    }
}

/// 协议命令常量
pub mod cmd {
    pub const LIST: &str = "list";
    pub const HASH: &str = "hash";
    pub const READ: &str = "read";
    pub const QUIT: &str = "quit";
}

pub const PROTOCOL_VERSION: u32 = 1;
