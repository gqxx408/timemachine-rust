use thiserror::Error;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("数据库错误: {0}")]
    Database(#[from] sqlx::Error),

    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("目标盘空间不足: 需要 {needed} 字节, 最大可用 {available} 字节")]
    InsufficientSpace { needed: u64, available: u64 },

    #[error("文件不存在: {0}")]
    FileNotFound(String),

    #[error("哈希校验失败: 文件 {path} 期望 {expected} 实际 {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("网络错误: {0}")]
    Network(String),

    #[error("无可用备份目标盘")]
    NoAvailableTarget,

    #[error("备份会话 {session_id} 状态异常: {reason}")]
    SessionState { session_id: i64, reason: String },
}

pub type Result<T> = std::result::Result<T, BackupError>;
