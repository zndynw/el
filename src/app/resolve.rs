use crate::cli::{ExportArgs, ImportArgs};
use crate::config::{
    CompressionType, Config, DatabaseConfig, ErrorStrategy, ExportConfig, ExportFormat,
    ImportConfig, ImportFormat, LoggingConfig, TransactionMode,
};
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use super::validate::validate_import_target;

#[derive(Debug)]
pub(crate) struct ResolvedImportConfig {
    pub(crate) config_path: Option<String>,
    pub(crate) database: DatabaseConfig,
    pub(crate) import: ImportConfig,
    pub(crate) logging: LoggingConfig,
}

#[derive(Debug)]
pub(crate) struct ResolvedExportConfig {
    pub(crate) config_path: Option<String>,
    pub(crate) database: DatabaseConfig,
    pub(crate) export: ExportConfig,
    pub(crate) logging: LoggingConfig,
}

pub(crate) fn resolve_import_config(
    args: ImportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<ResolvedImportConfig> {
    if let Some(config_path) = args.config.clone() {
        let cfg = Config::from_file(&config_path)?;
        let resolved_vars = merge_template_vars(cfg.vars, vars_override);
        let database = merge_database_config_import(cfg.database, &args);
        let import = merge_import_config(
            cfg.import
                .unwrap_or_else(|| build_import_config_from_args(&args).unwrap()),
            &args,
        )?;
        let import = apply_import_templates(import, &resolved_vars)?;
        let logging =
            merge_logging_config_import(cfg.logging, &args, verbose_override, log_tag_override);

        return Ok(ResolvedImportConfig {
            config_path: Some(config_path),
            database,
            import,
            logging,
        });
    }

    Ok(ResolvedImportConfig {
        config_path: None,
        database: build_database_config_from_args_import(&args)?,
        import: apply_import_templates(build_import_config_from_args(&args)?, &vars_override)?,
        logging: merge_logging_config_import(
            LoggingConfig::default(),
            &args,
            verbose_override,
            log_tag_override,
        ),
    })
}

pub(crate) fn merge_database_config_import(
    mut config: DatabaseConfig,
    args: &ImportArgs,
) -> DatabaseConfig {
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
    } else if (config.db_type == "postgresql" || config.db_type == "greenplum")
        && config.password.is_empty()
    {
        if let Ok(pgpassword) = std::env::var("PGPASSWORD") {
            config.password = pgpassword;
        }
    }
    if let Some(host) = &args.gpfdist_host {
        config.gpfdist_host = Some(host.clone());
    }
    if let Some(port) = args.gpfdist_port {
        config.gpfdist_port = Some(port);
    }
    if let Some(dir) = &args.gpfdist_dir {
        config.gpfdist_dir = Some(dir.clone());
    }

    config
}

pub(crate) fn merge_import_config(
    mut config: ImportConfig,
    args: &ImportArgs,
) -> Result<ImportConfig> {
    if let Some(schema) = &args.schema {
        config.schema = Some(schema.clone());
    }
    if let Some(table) = &args.table {
        config.table = table.clone();
    }
    if let Some(input) = &args.input {
        config.input_file = input.clone();
    }
    if let Some(source_columns) = &args.source_columns {
        config.source_columns = Some(
            source_columns
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
        );
    }
    if let Some(target_columns) = &args.target_columns {
        config.target_columns = Some(
            target_columns
                .split(',')
                .map(|s| s.trim().to_string())
                .collect(),
        );
    }
    if let Some(mapping) = &args.column_mapping {
        config.column_mapping = Some(parse_column_mapping(mapping)?);
    }
    if let Some(skip) = &args.skip_columns {
        config.skip_columns = Some(skip.split(',').map(|s| s.trim().to_string()).collect());
    }
    if let Some(types) = &args.column_types {
        config.column_types = Some(parse_column_types(types)?);
    }
    if let Some(format) = &args.format {
        config.format = format.parse().map_err(|e: String| anyhow!(e))?;
    }
    if let Some(delimiter) = &args.delimiter {
        config.delimiter = delimiter.clone();
    }
    if let Some(escape) = &args.escape {
        config.escape = Some(escape.clone());
    }
    if let Some(show_progress) = args.progress_override() {
        config.show_progress = show_progress;
    }
    if let Some(has_header) = args.header_override() {
        config.has_header = has_header;
    }
    if let Some(batch_size) = args.batch_size {
        config.batch_size = batch_size;
    }
    if let Some(null_value) = &args.null_value {
        config.null_value = null_value.clone();
    }
    if let Some(on_error) = &args.on_error {
        config.on_error = on_error.parse().map_err(|e: String| anyhow!(e))?;
    }
    if let Some(transaction) = &args.transaction {
        config.transaction_mode = transaction.parse().map_err(|e: String| anyhow!(e))?;
    }
    if args.truncate {
        config.truncate_table = true;
    }
    if let Some(pre_sql) = &args.pre_sql {
        config.pre_sql = Some(pre_sql.clone());
    }
    if let Some(post_sql) = &args.post_sql {
        config.post_sql = Some(post_sql.clone());
    }
    if let Some(error_log_table) = &args.error_log_table {
        config.error_log_table = Some(error_log_table.clone());
    }
    if let Some(compression) = &args.compression {
        config.compression = parse_compression_type(compression)?;
    }
    if let Some(progress_interval_secs) = args.progress_interval_secs {
        config.progress_interval_secs = progress_interval_secs;
    }

    validate_import_target(&config)?;

    Ok(config)
}

pub(crate) fn merge_logging_config_import(
    mut config: LoggingConfig,
    args: &ImportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
) -> LoggingConfig {
    if let Some(log_file) = &args.log_file {
        config.log_file = Some(log_file.clone());
    }
    if let Some(log_tag) = log_tag_override {
        config.tag = Some(log_tag);
    }
    if let Some(verbose) = verbose_override {
        config.verbose = verbose;
    }

    config
}

pub(crate) fn build_database_config_from_args_import(args: &ImportArgs) -> Result<DatabaseConfig> {
    let db_type = args.db_type.clone().context("--db-type is required")?;
    let password = if let Some(pwd) = &args.password {
        pwd.clone()
    } else if args.dry_run || args.print_resolved_config {
        String::new()
    } else if db_type == "postgresql" || db_type == "greenplum" {
        std::env::var("PGPASSWORD").unwrap_or_default()
    } else {
        return Err(anyhow!("--password is required"));
    };

    Ok(DatabaseConfig {
        db_type,
        connection_string: args.conn.clone().context("--conn is required")?,
        username: args.username.clone().context("--username is required")?,
        password,
        fetch_size: 1000,
        gpfdist_host: args.gpfdist_host.clone(),
        gpfdist_port: args.gpfdist_port,
        gpfdist_dir: args.gpfdist_dir.clone(),
    })
}

pub(crate) fn build_import_config_from_args(args: &ImportArgs) -> Result<ImportConfig> {
    let config = ImportConfig {
        schema: args.schema.clone(),
        table: args.table.clone().context("--table is required")?,
        input_file: args.input.clone().context("--input is required")?,
        source_columns: args
            .source_columns
            .as_ref()
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect()),
        target_columns: args
            .target_columns
            .as_ref()
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect()),
        column_mapping: args
            .column_mapping
            .as_ref()
            .map(|s| parse_column_mapping(s))
            .transpose()?,
        column_expressions: None,
        skip_columns: args
            .skip_columns
            .as_ref()
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect()),
        column_types: args
            .column_types
            .as_ref()
            .map(|s| parse_column_types(s))
            .transpose()?,
        format: args
            .format
            .as_ref()
            .map(|s| s.parse().map_err(|e: String| anyhow!(e)))
            .transpose()?
            .unwrap_or(ImportFormat::Csv),
        delimiter: args.delimiter.clone().unwrap_or_else(|| ",".to_string()),
        escape: args.escape.clone(),
        has_header: args.header_override().unwrap_or(true),
        batch_size: args.batch_size.unwrap_or(1000),
        null_value: args.null_value.clone().unwrap_or_default(),
        on_error: args
            .on_error
            .as_ref()
            .map(|s| s.parse().map_err(|e: String| anyhow!(e)))
            .transpose()?
            .unwrap_or(ErrorStrategy::Skip),
        transaction_mode: args
            .transaction
            .as_ref()
            .map(|s| s.parse().map_err(|e: String| anyhow!(e)))
            .transpose()?
            .unwrap_or(TransactionMode::PerBatch),
        show_progress: args.progress_override().unwrap_or(false),
        progress_interval_secs: args.progress_interval_secs.unwrap_or(30),
        truncate_table: args.truncate,
        pre_sql: args.pre_sql.clone(),
        post_sql: args.post_sql.clone(),
        error_log_table: args.error_log_table.clone(),
        compression: args
            .compression
            .as_ref()
            .map(|s| parse_compression_type(s))
            .transpose()?
            .unwrap_or(CompressionType::None),
    };

    validate_import_target(&config)?;

    Ok(config)
}

fn parse_column_mapping(s: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in s.split(',') {
        let parts: Vec<_> = pair.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid column mapping format: {}", pair));
        }
        map.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
    }
    Ok(map)
}

fn parse_column_types(s: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for pair in s.split(',') {
        let parts: Vec<_> = pair.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid column types format: {}", pair));
        }
        map.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
    }
    Ok(map)
}

pub(crate) fn resolve_export_config(
    args: ExportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<ResolvedExportConfig> {
    if let Some(config_path) = args.config.clone() {
        let cfg = Config::from_file(&config_path)?;
        let resolved_vars = merge_template_vars(cfg.vars, vars_override);
        let database = merge_database_config(cfg.database, &args);
        let export = merge_export_config(
            cfg.export
                .unwrap_or_else(|| build_export_config_from_args(&args).unwrap()),
            &args,
        )?;
        let export = apply_export_templates(export, &resolved_vars)?;
        let logging = merge_logging_config(cfg.logging, &args, verbose_override, log_tag_override);

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
        export: apply_export_templates(build_export_config_from_args(&args)?, &vars_override)?,
        logging: merge_logging_config(
            LoggingConfig::default(),
            &args,
            verbose_override,
            log_tag_override,
        ),
    })
}

pub(crate) fn merge_database_config(
    mut config: DatabaseConfig,
    args: &ExportArgs,
) -> DatabaseConfig {
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
    } else if (config.db_type == "postgresql" || config.db_type == "greenplum")
        && config.password.is_empty()
    {
        if let Ok(pgpassword) = std::env::var("PGPASSWORD") {
            config.password = pgpassword;
        }
    }
    if let Some(fetch) = args.fetch {
        config.fetch_size = fetch;
    }

    config
}

pub(crate) fn merge_export_config(
    mut config: ExportConfig,
    args: &ExportArgs,
) -> Result<ExportConfig> {
    if let Some(query) = &args.query {
        config.query = query.clone();
    }
    if let Some(output) = &args.output {
        config.output_file = output.clone();
    }
    if let Some(format) = &args.format {
        config.format = parse_export_format(format)?;
    }
    if let Some(delimiter) = &args.delimiter {
        config.delimiter = delimiter.clone();
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
    if let Some(progress_interval_secs) = args.progress_interval_secs {
        config.progress_interval_secs = progress_interval_secs;
    }
    if let Some(count_rows) = args.count_rows_override() {
        config.count_rows = count_rows;
    }

    Ok(config)
}

pub(crate) fn merge_logging_config(
    mut config: LoggingConfig,
    args: &ExportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
) -> LoggingConfig {
    if let Some(log_file) = &args.log_file {
        config.log_file = Some(log_file.clone());
    }
    if let Some(log_tag) = log_tag_override {
        config.tag = Some(log_tag);
    }
    if let Some(verbose) = verbose_override {
        config.verbose = verbose;
    }

    config
}

pub(crate) fn build_database_config_from_args(args: &ExportArgs) -> Result<DatabaseConfig> {
    let db_type = args.db_type.clone().unwrap_or_else(|| "oracle".to_string());
    let non_executing_mode = args.dry_run || args.print_resolved_config;
    let password = if let Some(pwd) = &args.password {
        pwd.clone()
    } else if non_executing_mode {
        String::new()
    } else if db_type == "postgresql" || db_type == "greenplum" {
        std::env::var("PGPASSWORD").unwrap_or_else(|_| String::new())
    } else {
        required_arg(&args.password, "Password")?
    };

    if password.is_empty()
        && !non_executing_mode
        && (db_type == "postgresql" || db_type == "greenplum")
    {
        return Err(anyhow!(
            "Password is required. Use --password or set PGPASSWORD environment variable"
        ));
    }

    Ok(DatabaseConfig {
        db_type,
        connection_string: required_arg(&args.conn, "Connection string")?,
        username: required_arg(&args.username, "Username")?,
        password,
        fetch_size: args.fetch.unwrap_or(1000),
        gpfdist_host: None,
        gpfdist_port: None,
        gpfdist_dir: None,
    })
}

fn build_export_config_from_args(args: &ExportArgs) -> Result<ExportConfig> {
    let query = required_arg(&args.query, "Query")?;
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
        include_header: args.header_override().unwrap_or(false),
        buffer_size: args.buffer_size.unwrap_or(1024 * 1024),
        compression,
        progress_interval_secs: args.progress_interval_secs.unwrap_or(30),
        skip_errors: false,
        count_rows: args.count_rows_override().unwrap_or(false),
    })
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

pub(crate) fn parse_cli_vars(raw_vars: &[String]) -> Result<HashMap<String, String>> {
    let mut vars = HashMap::new();
    for entry in raw_vars {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid --var format '{}', expected key=value", entry))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!(
                "invalid --var format '{}', variable name is empty",
                entry
            ));
        }
        vars.insert(key.to_string(), value.to_string());
    }
    Ok(vars)
}

fn merge_template_vars(
    mut config_vars: HashMap<String, String>,
    cli_vars: HashMap<String, String>,
) -> HashMap<String, String> {
    config_vars.extend(cli_vars);
    config_vars
}

fn apply_import_templates(
    mut config: ImportConfig,
    vars: &HashMap<String, String>,
) -> Result<ImportConfig> {
    config.input_file = render_template(&config.input_file, vars, &HashSet::new())?;
    if let Some(pre_sql) = &config.pre_sql {
        config.pre_sql = Some(render_template(
            pre_sql,
            vars,
            &HashSet::from(["ext_table"]),
        )?);
    }
    if let Some(post_sql) = &config.post_sql {
        config.post_sql = Some(render_template(
            post_sql,
            vars,
            &HashSet::from(["ext_table"]),
        )?);
    }
    Ok(config)
}

pub(crate) fn apply_export_templates(
    mut config: ExportConfig,
    vars: &HashMap<String, String>,
) -> Result<ExportConfig> {
    config.query = resolve_export_query(&config.query, vars)?;
    config.output_file = render_template(&config.output_file, vars, &HashSet::new())?;
    Ok(config)
}

pub(crate) fn resolve_export_query(input: &str, vars: &HashMap<String, String>) -> Result<String> {
    let rendered_input = render_template(input, vars, &HashSet::new())?;
    let query = read_query_or_file(&rendered_input)?;

    if query == rendered_input {
        Ok(query)
    } else {
        render_template(&query, vars, &HashSet::new())
    }
}

pub(crate) fn render_template(
    input: &str,
    vars: &HashMap<String, String>,
    allowed_unresolved: &HashSet<&str>,
) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let end = after_start
            .find('}')
            .ok_or_else(|| anyhow!("unclosed template variable in '{}'", input))?;
        let key = &after_start[..end];
        if key.is_empty() {
            return Err(anyhow!("empty template variable in '{}'", input));
        }
        if let Some(value) = vars.get(key) {
            output.push_str(value);
        } else if allowed_unresolved.contains(key) {
            output.push('{');
            output.push_str(key);
            output.push('}');
        } else {
            return Err(anyhow!("missing template variable: {}", key));
        }
        rest = &after_start[end + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

pub(crate) fn export_format_name(format: &ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "csv",
        ExportFormat::Tsv => "tsv",
        ExportFormat::Custom => "custom",
    }
}

pub(crate) fn import_format_name(format: &ImportFormat) -> &'static str {
    match format {
        ImportFormat::Csv => "csv",
        ImportFormat::Tsv => "tsv",
        ImportFormat::Custom => "custom",
    }
}

pub(crate) fn error_strategy_name(strategy: &ErrorStrategy) -> &'static str {
    match strategy {
        ErrorStrategy::Skip => "skip",
        ErrorStrategy::Abort => "abort",
    }
}

pub(crate) fn transaction_mode_name(mode: &TransactionMode) -> &'static str {
    match mode {
        TransactionMode::PerBatch => "per_batch",
        TransactionMode::All => "all",
        TransactionMode::None => "none",
    }
}

pub(crate) fn compression_type_name(compression: &CompressionType) -> &'static str {
    match compression {
        CompressionType::None => "none",
        CompressionType::Gzip => "gzip",
    }
}
