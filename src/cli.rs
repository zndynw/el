use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "el")]
#[command(about = "数据导出导入工具 - Data Export/Import Tool", long_about = None)]
pub struct Cli {
    /// 详细日志 (Verbose logging)
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// 关闭详细日志 (Disable verbose logging)
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "verbose")]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn verbose_override(&self) -> Option<bool> {
        if self.verbose {
            Some(true)
        } else if self.quiet {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// 导出数据 (Export data)
    Export(ExportArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ExportArgs {
    /// 配置文件路径 (Config file path)
    #[arg(short, long)]
    pub config: Option<String>,

    /// 数据库类型 (Database type: oracle/mysql/postgresql)
    #[arg(long)]
    pub db_type: Option<String>,

    /// 数据库连接字符串 (Database connection string: host:port/service_name)
    #[arg(long)]
    pub conn: Option<String>,

    /// 用户名 (Username)
    #[arg(long)]
    pub username: Option<String>,

    /// 密码 (Password)
    #[arg(long)]
    pub password: Option<String>,

    /// 查询SQL或SQL文件路径 (Query SQL or SQL file path)
    #[arg(long)]
    pub query: Option<String>,

    /// 输出文件 (Output file)
    #[arg(short, long)]
    pub output: Option<String>,

    /// 导出格式 (Export format: csv/tsv/custom)
    #[arg(long)]
    pub format: Option<String>,

    /// 分隔符 (Delimiter)
    #[arg(long)]
    pub delimiter: Option<String>,

    /// 显示进度 (Show progress)
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_progress")]
    pub progress: bool,

    /// 不显示进度 (Do not show progress)
    #[arg(long = "no-progress", action = ArgAction::SetTrue, conflicts_with = "progress")]
    pub no_progress: bool,

    /// 批量获取大小 (Fetch size)
    #[arg(long)]
    pub fetch: Option<usize>,

    /// 包含表头 (Include header)
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_header")]
    pub header: bool,

    /// 不包含表头 (Do not include header)
    #[arg(long = "no-header", action = ArgAction::SetTrue, conflicts_with = "header")]
    pub no_header: bool,

    /// 缓冲区大小（字节） (Buffer size in bytes)
    #[arg(long)]
    pub buffer_size: Option<usize>,

    /// 压缩类型 (Compression type: none/gzip)
    #[arg(long)]
    pub compression: Option<String>,

    /// 日志文件路径 (Log file path, append mode)
    #[arg(long)]
    pub log_file: Option<String>,

    /// 进度输出间隔（行数）(Progress output interval in rows)
    #[arg(long)]
    pub progress_interval: Option<u64>,
}

impl ExportArgs {
    pub fn progress_override(&self) -> Option<bool> {
        if self.progress {
            Some(true)
        } else if self.no_progress {
            Some(false)
        } else {
            None
        }
    }

    pub fn header_override(&self) -> Option<bool> {
        if self.header {
            Some(true)
        } else if self.no_header {
            Some(false)
        } else {
            None
        }
    }
}
