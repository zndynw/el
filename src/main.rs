mod config;
mod db;
mod export;

use clap::{Parser, Subcommand};
use config::{CompressionType, Config, DatabaseConfig, ExportConfig, ExportFormat};
use db::oracle::OracleDatabase;
use db::Database;
use export::Exporter;
use anyhow::Result;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "el")]
#[command(about = "数据导出导入工具 - Data Export/Import Tool", long_about = None)]
struct Cli {
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
    },
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
        } => {
            let (db_config, export_config) = if let Some(config_path) = config {
                // 从配置文件加载
                println!("Loading configuration from: {}", config_path);
                let cfg = Config::from_file(&config_path)?;
                let mut exp_cfg = cfg.export;
                // 处理配置文件中的query字段，支持SQL文件路径
                exp_cfg.query = read_query_or_file(&exp_cfg.query)?;
                (cfg.database, exp_cfg)
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
                };

                (db_config, export_config)
            };

            // 执行导出
            println!("Connecting to {} database...", db_config.db_type);
            let mut db = OracleDatabase::new(db_config);
            db.connect()?;
            println!("Connected successfully!");

            println!("Starting export...");
            let mut exporter = Exporter::new(export_config);
            let stats = exporter.export(&mut db)?;

            stats.print_summary();
            println!("Export completed successfully!");

            Ok(())
        }
    }
}
