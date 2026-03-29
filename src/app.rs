use crate::cli::{Cli, Commands, ExportArgs};
use crate::config::{
    CompressionType, Config, DatabaseConfig, ExportConfig, ExportFormat, LoggingConfig,
};
use crate::db::Database;
use crate::db::mysql::MySqlDatabase;
use crate::db::oracle::OracleDatabase;
use crate::export::Exporter;
use crate::logging::init_tracing;
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::Path;
use tracing::info;

pub fn run(cli: Cli) -> Result<()> {
    let verbose_override = cli.verbose_override();

    match cli.command {
        Commands::Export(args) => run_export(args, verbose_override),
    }
}

fn run_export(args: ExportArgs, verbose_override: Option<bool>) -> Result<()> {
    let resolved = resolve_export_config(args, verbose_override)?;

    init_tracing(
        resolved.logging.log_file.as_deref(),
        resolved.logging.verbose,
    )?;

    if let Some(config_path) = &resolved.config_path {
        info!("Loading configuration from: {}", config_path);
    }

    tracing::debug!("Configuration Details:");
    tracing::debug!("  Database type: {}", resolved.database.db_type);
    tracing::debug!(
        "  Connection string: {}",
        resolved.database.connection_string
    );
    tracing::debug!("  Username: {}", resolved.database.username);
    tracing::debug!("  Fetch size: {}", resolved.database.fetch_size);
    tracing::debug!("  Output file: {}", resolved.export.output_file);
    tracing::debug!("  Format: {:?}", resolved.export.format);
    tracing::debug!("  Delimiter: {:?}", resolved.export.delimiter);
    tracing::debug!("  Show progress: {}", resolved.export.show_progress);
    tracing::debug!("  Include header: {}", resolved.export.include_header);
    tracing::debug!("  Buffer size: {} bytes", resolved.export.buffer_size);
    tracing::debug!("  Compression: {:?}", resolved.export.compression);
    tracing::debug!("Query SQL:");
    tracing::debug!("{}", resolved.export.query);

    info!("Connecting to {} database...", resolved.database.db_type);
    let mut db = build_database(resolved.database)?;
    db.connect()?;
    info!("Connected successfully!");

    info!("Starting export...");
    let mut exporter = Exporter::new(resolved.export);
    let stats = exporter.export(db.as_mut())?;

    stats.print_summary();
    info!("Export completed successfully!");

    Ok(())
}

struct ResolvedExportConfig {
    config_path: Option<String>,
    database: DatabaseConfig,
    export: ExportConfig,
    logging: LoggingConfig,
}

fn resolve_export_config(
    args: ExportArgs,
    verbose_override: Option<bool>,
) -> Result<ResolvedExportConfig> {
    if let Some(config_path) = args.config.clone() {
        let cfg = Config::from_file(&config_path)?;
        let database = merge_database_config(cfg.database, &args);
        let export = merge_export_config(cfg.export, &args)?;
        let logging = merge_logging_config(cfg.logging, &args, verbose_override);

        return Ok(ResolvedExportConfig {
            config_path: Some(config_path),
            database,
            export,
            logging,
        });
    }

    Ok(ResolvedExportConfig {
        config_path: None,
        database: build_database_config_from_args(&args)?,
        export: build_export_config_from_args(&args)?,
        logging: merge_logging_config(LoggingConfig::default(), &args, verbose_override),
    })
}

fn merge_database_config(mut config: DatabaseConfig, args: &ExportArgs) -> DatabaseConfig {
    if let Some(db_type) = &args.db_type {
        config.db_type = db_type.clone();
    }
    if let Some(conn) = &args.conn {
        config.connection_string = conn.clone();
    }
    if let Some(username) = &args.username {
        config.username = username.clone();
    }
    if let Some(password) = &args.password {
        config.password = password.clone();
    }
    if let Some(fetch) = args.fetch {
        config.fetch_size = fetch;
    }

    config
}

fn merge_export_config(mut config: ExportConfig, args: &ExportArgs) -> Result<ExportConfig> {
    config.query = if let Some(query) = &args.query {
        read_query_or_file(query)?
    } else {
        read_query_or_file(&config.query)?
    };

    if let Some(output) = &args.output {
        config.output_file = output.clone();
    }
    if let Some(format) = &args.format {
        config.format = parse_export_format(format)?;
    }
    if let Some(delimiter) = &args.delimiter {
        config.delimiter = delimiter.clone();
    }
    if let Some(show_progress) = args.progress_override() {
        config.show_progress = show_progress;
    }
    if let Some(include_header) = args.header_override() {
        config.include_header = include_header;
    }
    if let Some(buffer_size) = args.buffer_size {
        config.buffer_size = buffer_size;
    }
    if let Some(compression) = &args.compression {
        config.compression = parse_compression_type(compression)?;
    }
    if let Some(progress_interval) = args.progress_interval {
        config.progress_interval = progress_interval;
    }

    Ok(config)
}

fn merge_logging_config(
    mut config: LoggingConfig,
    args: &ExportArgs,
    verbose_override: Option<bool>,
) -> LoggingConfig {
    if let Some(log_file) = &args.log_file {
        config.log_file = Some(log_file.clone());
    }
    if let Some(verbose) = verbose_override {
        config.verbose = verbose;
    }

    config
}

fn build_database_config_from_args(args: &ExportArgs) -> Result<DatabaseConfig> {
    Ok(DatabaseConfig {
        db_type: args.db_type.clone().unwrap_or_else(|| "oracle".to_string()),
        connection_string: required_arg(&args.conn, "Connection string")?,
        username: required_arg(&args.username, "Username")?,
        password: required_arg(&args.password, "Password")?,
        fetch_size: args.fetch.unwrap_or(1000),
    })
}

fn build_export_config_from_args(args: &ExportArgs) -> Result<ExportConfig> {
    let query = read_query_or_file(&required_arg(&args.query, "Query")?)?;
    let format = match &args.format {
        Some(value) => parse_export_format(value)?,
        None => ExportFormat::Csv,
    };
    let compression = match &args.compression {
        Some(value) => parse_compression_type(value)?,
        None => CompressionType::None,
    };

    Ok(ExportConfig {
        query,
        output_file: required_arg(&args.output, "Output file")?,
        format,
        delimiter: args.delimiter.clone().unwrap_or_else(|| "\x03".to_string()),
        show_progress: args.progress_override().unwrap_or(false),
        include_header: args.header_override().unwrap_or(false),
        buffer_size: args.buffer_size.unwrap_or(1024 * 1024),
        compression,
        progress_interval: args.progress_interval.unwrap_or(1_000_000),
        skip_errors: false,
    })
}

fn build_database(config: DatabaseConfig) -> Result<Box<dyn Database>> {
    match config.db_type.to_lowercase().as_str() {
        "mysql" => Ok(Box::new(MySqlDatabase::new(config))),
        "oracle" => Ok(Box::new(OracleDatabase::new(config))),
        other => Err(anyhow!("Unsupported database type: {other}")),
    }
}

fn parse_export_format(value: &str) -> Result<ExportFormat> {
    value
        .parse()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid export format: {value}"))
}

fn parse_compression_type(value: &str) -> Result<CompressionType> {
    value
        .parse()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid compression type: {value}"))
}

fn read_query_or_file(input: &str) -> Result<String> {
    let path = Path::new(input);

    if path.exists() && path.is_file() {
        let content = fs::read_to_string(path)?;
        Ok(content.trim().to_string())
    } else {
        Ok(input.to_string())
    }
}

fn required_arg(value: &Option<String>, name: &str) -> Result<String> {
    value.clone().ok_or_else(|| anyhow!("{name} is required"))
}

#[cfg(test)]
mod tests {
    use super::{merge_database_config, merge_export_config};
    use crate::cli::ExportArgs;
    use crate::config::{CompressionType, DatabaseConfig, ExportConfig, ExportFormat};

    fn empty_args() -> ExportArgs {
        ExportArgs {
            config: None,
            db_type: None,
            conn: None,
            username: None,
            password: None,
            query: None,
            output: None,
            format: None,
            delimiter: None,
            progress: false,
            no_progress: false,
            fetch: None,
            header: false,
            no_header: false,
            buffer_size: None,
            compression: None,
            log_file: None,
            progress_interval: None,
        }
    }

    #[test]
    fn merge_database_config_overrides_only_explicit_cli_values() {
        let base = DatabaseConfig {
            db_type: "oracle".to_string(),
            connection_string: "db:1521/ORCL".to_string(),
            username: "scott".to_string(),
            password: "tiger".to_string(),
            fetch_size: 500,
        };
        let mut args = empty_args();
        args.fetch = Some(2000);
        args.username = Some("new-user".to_string());

        let merged = merge_database_config(base, &args);

        assert_eq!(merged.db_type, "oracle");
        assert_eq!(merged.connection_string, "db:1521/ORCL");
        assert_eq!(merged.username, "new-user");
        assert_eq!(merged.password, "tiger");
        assert_eq!(merged.fetch_size, 2000);
    }

    #[test]
    fn merge_export_config_preserves_config_values_without_cli_overrides() {
        let base = ExportConfig {
            query: "SELECT 1".to_string(),
            output_file: "output.csv".to_string(),
            format: ExportFormat::Csv,
            delimiter: ",".to_string(),
            show_progress: false,
            include_header: false,
            buffer_size: 1024,
            compression: CompressionType::None,
            progress_interval: 10,
            skip_errors: false,
        };
        let args = empty_args();

        let merged = merge_export_config(base, &args).expect("merge should succeed");

        assert_eq!(merged.query, "SELECT 1");
        assert_eq!(merged.output_file, "output.csv");
        assert_eq!(merged.format, ExportFormat::Csv);
        assert!(!merged.show_progress);
        assert!(!merged.include_header);
        assert_eq!(merged.compression, CompressionType::None);
        assert_eq!(merged.progress_interval, 10);
    }

    #[test]
    fn merge_export_config_allows_disabling_progress_from_cli() {
        let base = ExportConfig {
            query: "SELECT 1".to_string(),
            output_file: "output.csv".to_string(),
            format: ExportFormat::Csv,
            delimiter: ",".to_string(),
            show_progress: true,
            include_header: true,
            buffer_size: 1024,
            compression: CompressionType::None,
            progress_interval: 10,
            skip_errors: false,
        };
        let mut args = empty_args();
        args.no_progress = true;
        args.no_header = true;

        let merged = merge_export_config(base, &args).expect("merge should succeed");

        assert!(!merged.show_progress);
        assert!(!merged.include_header);
    }

    #[test]
    fn merge_logging_config_allows_disabling_verbose_from_cli() {
        let logging = super::merge_logging_config(
            crate::config::LoggingConfig {
                log_file: None,
                verbose: true,
            },
            &empty_args(),
            Some(false),
        );

        assert!(!logging.verbose);
    }
}
