use sqlx::FromRow;
use std::path::PathBuf;

#[derive(Debug, Clone, FromRow)]
pub struct BackupRoot {
    pub id: i64,
    pub root_path: String,
    pub source_type: String,
    pub label: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct BackupTarget {
    pub id: i64,
    pub target_path: String,
    pub subdir_name: String,
    pub max_quota: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct ContentBlock {
    pub hash: String,
    pub hash_algo: String,
    pub file_size: i64,
    pub target_path: String,
    pub target_id: i64,
    pub ref_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct FileVersion {
    pub id: i64,
    pub tracked_file_id: i64,
    pub session_id: i64,
    pub content_hash: String,
    pub mtime: i64,
    pub file_size: i64,
}

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HashAlgo {
    Blake3,
    Md5,
}

impl HashAlgo {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blake3 => "blake3",
            Self::Md5 => "md5",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "md5" => Self::Md5,
            _ => Self::Blake3,
        }
    }
}

/// 备份源类型: 本地或远程
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceType {
    Local,
    Remote,
}

impl SourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "remote" => Self::Remote,
            _ => Self::Local,
        }
    }
}
