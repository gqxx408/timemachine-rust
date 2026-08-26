use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "tm-rust",
    version,
    about = "TimeMachine Plus - Rust 增量备份系统 (支持局域网远程备份 + 多盘负载均衡)"
)]
pub struct Cli {
    #[arg(long, default_value = "config.toml", global = true, help = "配置文件路径")]
    pub config: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// 初始化数据库
    Init,

    /// 执行备份
    Backup,

    /// 数据完整性校验
    Check {
        #[arg(long, help = "校验文件哈希(较慢)")]
        with_hash: bool,
    },

    /// 删除指定备份源的所有数据
    Gc {
        #[arg(long, help = "备份源 ID")]
        root_id: i64,
    },

    /// 添加备份源 (本地目录或远程地址 host:port)
    AddRoot {
        #[arg(long, help = "路径 (local) 或地址 host:port (remote)")]
        path: String,
        #[arg(long, default_value = "local", help = "local 或 remote")]
        source_type: String,
        #[arg(long, help = "可读标签")]
        label: Option<String>,
    },

    /// 添加备份目标盘
    AddTarget {
        #[arg(long, help = "目标盘路径")]
        path: String,
        #[arg(long, default_value = "BACKUPDATABASE", help = "子目录名")]
        subdir: String,
        #[arg(long, help = "配额限制 (bytes), 不填则不限制")]
        quota: Option<i64>,
    },

    /// 列出所有备份源和目标盘
    List,

    /// 启动网络服务端 (被其他电脑拉取备份)
    Server {
        #[arg(long, help = "暴露的根目录路径")]
        root: String,
        #[arg(long, help = "监听地址, 默认用配置文件")]
        addr: Option<String>,
    },

    /// 测试远程服务端连接
    Ping {
        #[arg(long, help = "远程地址 host:port")]
        addr: String,
    },

    /// 启动图形界面
    Gui,
}
