mod config;
mod db;
mod export;

use clap::{Parser, Subcommand};
use config::{CompressionType, Config, DatabaseConfig, ExportConfig, ExportFormat, LoggingConfig};
use db::oracle::OracleDatabase;
use db::Database;
use export::Exporter;
use anyhow::Result;
use std::fs;
use std::path::Path;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "el")]
#[command(about = "数据导出导入工具 - Data Export/Import Tool", long_about = None)]
struct Cli {
    /// 详细日志 (Verbose logging)
    #[arg(short, long, global = true)]
    verbose: bool,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 导出数据 (Export data)
    Export {
        /// 配置文件路径 (Config file path)
        #[arg(short, long)]
        config: Option<String>,

        /// 数据库类型 (Database type: oracle/mysql/postgresql)
        #[arg(long)]
        db_type: Option<String>,

        /// 数据库连接字符串 (Database connection string: host:port/service_name)
        #[arg(long)]
        conn: Option<String>,

        /// 用户名 (Username)
        #[arg(long)]
        username: Option<String>,

        /// 密码 (Password)
        #[arg(long)]
        password: Option<String>,

        /// 查询SQL或SQL文件路径 (Query SQL or SQL file path)
        #[arg(long)]
        query: Option<String>,

        /// 输出文件 (Output file)
        #[arg(short, long)]
        output: Option<String>,

        /// 导出格式 (Export format: csv/tsv/custom)
        #[arg(long, default_value = "csv")]
        format: String,

        /// 分隔符 (Delimiter)
        #[arg(long)]
        delimiter: Option<String>,

        /// 显示进度 (Show progress)
        #[arg(long, default_value = "false")]
        progress: bool,

        /// 批量获取大小 (Fetch size)
        #[arg(long, default_value = "1000")]
        fetch: usize,

        /// 包含表头 (Include header)
        #[arg(long, default_value = "false")]
        header: bool,

        /// 缓冲区大小（字节）(Buffer size in bytes)
        #[arg(long, default_value = "1048576")]
        buffer_size: usize,

        /// 压缩类型 (Compression type: none/gzip)
        #[arg(long, default_value = "none")]
        compression: String,

        /// 日志文件路径 (Log file path, append mode)
        #[arg(long)]
        log_file: Option<String>,

        /// 进度输出间隔（行数）(Progress output interval in rows)
        #[arg(long, default_value = "1000000")]
        progress_interval: u64,
    },
}

/// 初始化tracing日志系统
fn init_tracing(log_file: Option<&String>, verbose: bool) -> Result<()> {
    let level = if verbose { "debug" } else { "info" };
    
    // 优先使用环境变量，如果没有设置则使用verbose参数
    let env_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
    } else {
        EnvFilter::new(level)
    };
    
    if let Some(log_path) = log_file {
        // 输出到文件（追加模式）
        let file_appender = tracing_appender::rolling::never(
            std::path::Path::new(log_path).parent().unwrap_or(std::path::Path::new(".")),
            std::path::Path::new(log_path).file_name().unwrap_or(std::ffi::OsStr::new("export.log"))
        );
        
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(file_appender).with_ansi(false))
            .init();
    } else {
        // 输出到控制台
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer())
            .init();
    }
    
    Ok(())
}

/// 读取SQL查询，支持直接传入SQL字符串或SQL文件路径
fn read_query_or_file(input: &str) -> Result<String> {
    let path = Path::new(input);
    
    // 检查是否为文件路径
    if path.exists() && path.is_file() {
        // 读取文件内容
        let content = fs::read_to_string(path)?;
        Ok(content.trim().to_string())
    } else {
        // 直接返回SQL字符串
        Ok(input.to_string())
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Export {
            config,
            db_type,
            conn,
            username,
            password,
            query,
            output,
            format,
            delimiter,
            progress,
            fetch,
            header,
            buffer_size,
            compression,
            log_file,
            progress_interval,
        } => {
            let (db_config, export_config, logging_config) = if let Some(ref config_path) = config {
                // 从配置文件加载
                let cfg = Config::from_file(config_path)?;
                let mut exp_cfg = cfg.export;
                // 处理配置文件中的query字段，支持SQL文件路径
                exp_cfg.query = read_query_or_file(&exp_cfg.query)?;
                
                // 命令行参数优先级高于配置文件
                let mut db_cfg = cfg.database;
                let mut log_cfg = cfg.logging;
                
                // 覆盖数据库配置
                if let Some(ref dt) = db_type {
                    db_cfg.db_type = dt.clone();
                }
                if let Some(ref c) = conn {
                    db_cfg.connection_string = c.clone();
                }
                if let Some(ref u) = username {
                    db_cfg.username = u.clone();
                }
                if let Some(ref p) = password {
                    db_cfg.password = p.clone();
                }
                if fetch != 1000 {  // 如果不是默认值，则覆盖
                    db_cfg.fetch_size = fetch;
                }
                
                // 覆盖导出配置
                if let Some(ref q) = query {
                    exp_cfg.query = read_query_or_file(q)?;
                }
                if let Some(ref o) = output {
                    exp_cfg.output_file = o.clone();
                }
                if format != "csv" {  // 如果不是默认值，则覆盖
                    exp_cfg.format = match format.to_lowercase().as_str() {
                        "csv" => ExportFormat::Csv,
                        "tsv" => ExportFormat::Tsv,
                        "custom" => ExportFormat::Custom,
                        _ => exp_cfg.format,
                    };
                }
                if let Some(ref d) = delimiter {
                    exp_cfg.delimiter = d.clone();
                }
                if progress {  // 如果命令行指定了progress，则覆盖
                    exp_cfg.show_progress = true;
                }
                if header {  // 如果命令行指定了header，则覆盖
                    exp_cfg.include_header = true;
                }
                if buffer_size != 1048576 {  // 如果不是默认值，则覆盖
                    exp_cfg.buffer_size = buffer_size;
                }
                if compression != "none" {  // 如果不是默认值，则覆盖
                    exp_cfg.compression = match compression.to_lowercase().as_str() {
                        "gzip" => CompressionType::Gzip,
                        _ => exp_cfg.compression,
                    };
                }
                if progress_interval != 1000000 {  // 如果不是默认值，则覆盖
                    exp_cfg.progress_interval = progress_interval;
                }
                
                // 覆盖日志配置
                if log_file.is_some() {
                    log_cfg.log_file = log_file;
                }
                if cli.verbose {
                    log_cfg.verbose = true;
                }
                
                (db_cfg, exp_cfg, log_cfg)
            } else {
                // 从命令行参数构建配置
                let db_config = DatabaseConfig {
                    db_type: db_type.unwrap_or_else(|| "oracle".to_string()),
                    connection_string: conn.ok_or_else(|| anyhow::anyhow!("Connection string is required"))?,
                    username: username.ok_or_else(|| anyhow::anyhow!("Username is required"))?,
                    password: password.ok_or_else(|| anyhow::anyhow!("Password is required"))?,
                    fetch_size: fetch,
                };

                let export_format = match format.to_lowercase().as_str() {
                    "csv" => ExportFormat::Csv,
                    "tsv" => ExportFormat::Tsv,
                    "custom" => ExportFormat::Custom,
                    _ => ExportFormat::Csv,
                };

                let query_input = query.ok_or_else(|| anyhow::anyhow!("Query is required"))?;
                let query_sql = read_query_or_file(&query_input)?;
                
                let compression_type = match compression.to_lowercase().as_str() {
                    "gzip" => CompressionType::Gzip,
                    _ => CompressionType::None,
                };
                
                let export_config = ExportConfig {
                    query: query_sql,
                    output_file: output.ok_or_else(|| anyhow::anyhow!("Output file is required"))?,
                    format: export_format,
                    delimiter: delimiter.unwrap_or_else(|| "\x03".to_string()),
                    show_progress: progress,
                    include_header: header,
                    buffer_size,
                    compression: compression_type,
                    progress_interval,
                };

                let logging_config = LoggingConfig {
                    log_file,
                    verbose: cli.verbose,
                };

                (db_config, export_config, logging_config)
            };

            // 初始化tracing
            init_tracing(logging_config.log_file.as_ref(), logging_config.verbose)?;
            
            if let Some(ref config_path) = config {
                info!("Loading configuration from: {}", config_path);
            }

            // 输出配置信息（verbose模式）
            tracing::debug!("Configuration Details:");
            tracing::debug!("  Database type: {}", db_config.db_type);
            tracing::debug!("  Connection string: {}", db_config.connection_string);
            tracing::debug!("  Username: {}", db_config.username);
            tracing::debug!("  Fetch size: {}", db_config.fetch_size);
            tracing::debug!("  Output file: {}", export_config.output_file);
            tracing::debug!("  Format: {:?}", export_config.format);
            tracing::debug!("  Delimiter: {:?}", export_config.delimiter);
            tracing::debug!("  Show progress: {}", export_config.show_progress);
            tracing::debug!("  Include header: {}", export_config.include_header);
            tracing::debug!("  Buffer size: {} bytes", export_config.buffer_size);
            tracing::debug!("  Compression: {:?}", export_config.compression);
            
            // 输出SQL脚本内容（verbose模式）
            tracing::debug!("Query SQL:");
            tracing::debug!("{}", export_config.query);

            // 执行导出
            info!("Connecting to {} database...", db_config.db_type);
            let mut db = OracleDatabase::new(db_config);
            db.connect()?;
            info!("Connected successfully!");

            info!("Starting export...");
            let mut exporter = Exporter::new(export_config);
            let stats = exporter.export(&mut db)?;

            stats.print_summary();
            info!("Export completed successfully!");

            Ok(())
        }
    }
}
