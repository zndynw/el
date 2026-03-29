use crate::cli::{Cli, Commands, ExportArgs, ImportArgs, InitArgs};
use crate::config::{
    CompressionType, Config, DatabaseConfig, ExportConfig, ExportFormat, ImportConfig, LoggingConfig,
};
use crate::db::Database;
use crate::db::greenplum::GreenplumDatabase;
use crate::db::mysql::MySqlDatabase;
use crate::db::oracle::OracleDatabase;
use crate::db::postgresql::PostgreSqlDatabase;
use crate::export::Exporter;
use crate::import::Importer;
use crate::logging::init_tracing;
use crate::templates;
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tracing::info;

pub fn run(cli: Cli) -> Result<()> {
    let verbose_override = cli.verbose_override();
    let log_tag_override = cli.log_tag.clone();
    let vars_override = parse_cli_vars(&cli.vars)?;

    match cli.command {
        Commands::Export(args) => run_export(args, verbose_override, log_tag_override, vars_override),
        Commands::Import(args) => run_import(args, verbose_override, log_tag_override, vars_override),
        Commands::Init(args) => run_init(args),
    }
}

fn run_init(args: InitArgs) -> Result<()> {
    if args.list {
        for template in templates::all() {
            println!("{}\t{}", template.id, template.description);
        }
        return Ok(());
    }

    let template_id = if let Some(template) = &args.template {
        template.clone()
    } else {
        let db_type = args
            .db_type
            .as_deref()
            .context("--template or the combination of --db-type and --mode is required")?;
        let mode = args
            .mode
            .as_deref()
            .context("--template or the combination of --db-type and --mode is required")?;
        templates::resolve_shortcut(db_type, mode)
            .ok_or_else(|| anyhow!("unsupported template shortcut: {} + {}", db_type, mode))?
            .to_string()
    };

    let template = templates::get(&template_id)
        .ok_or_else(|| anyhow!("unknown template: {}", template_id))?;
    let output = args
        .output
        .as_deref()
        .context("--output is required unless --list is used")?;
    let output_path = Path::new(output);

    if output_path.exists() && !args.force {
        return Err(anyhow!(
            "output file already exists: {} (use --force to overwrite)",
            output
        ));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(output_path, template.content)?;
    println!("Wrote template '{}' to {}", template.id, output);
    Ok(())
}

fn run_export(
    args: ExportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<()> {
    let resolved = resolve_export_config(args, verbose_override, log_tag_override, vars_override)?;

    init_tracing(
        resolved.logging.log_file.as_deref(),
        resolved.logging.tag.as_deref(),
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

fn run_import(
    args: ImportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<()> {
    let resolved = resolve_import_config(args, verbose_override, log_tag_override, vars_override)?;

    init_tracing(
        resolved.logging.log_file.as_deref(),
        resolved.logging.tag.as_deref(),
        resolved.logging.verbose,
    )?;

    if let Some(config_path) = &resolved.config_path {
        info!("Loading configuration from: {}", config_path);
    }

    info!("Connecting to {} database...", resolved.database.db_type);
    let mut db = build_database(resolved.database)?;
    db.connect()?;
    info!("Connected successfully!");

    info!("Starting import...");
    let mut importer = Importer::new(db, resolved.import);
    importer.import()?;

    info!("Import completed successfully!");

    Ok(())
}

struct ResolvedImportConfig {
    config_path: Option<String>,
    database: DatabaseConfig,
    import: ImportConfig,
    logging: LoggingConfig,
}

fn resolve_import_config(
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
            cfg.import.unwrap_or_else(|| build_import_config_from_args(&args).unwrap()),
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

fn merge_database_config_import(mut config: DatabaseConfig, args: &ImportArgs) -> DatabaseConfig {
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
    } else if (config.db_type == "postgresql" || config.db_type == "greenplum") && config.password.is_empty() {
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

fn merge_import_config(mut config: ImportConfig, args: &ImportArgs) -> Result<ImportConfig> {
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
        config.source_columns = Some(source_columns.split(',').map(|s| s.trim().to_string()).collect());
    }
    if let Some(target_columns) = &args.target_columns {
        config.target_columns = Some(target_columns.split(',').map(|s| s.trim().to_string()).collect());
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
    if let Some(progress_interval) = args.progress_interval {
        config.progress_interval = progress_interval;
    }

    validate_import_target(&config)?;

    Ok(config)
}

fn merge_logging_config_import(
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

fn build_database_config_from_args_import(args: &ImportArgs) -> Result<DatabaseConfig> {
    let db_type = args.db_type.clone().context("--db-type is required")?;
    let password = if let Some(pwd) = &args.password {
        pwd.clone()
    } else if db_type == "postgresql" || db_type == "greenplum" {
        std::env::var("PGPASSWORD").unwrap_or_default()
    } else {
        return Err(anyhow::anyhow!("--password is required"));
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

fn build_import_config_from_args(args: &ImportArgs) -> Result<ImportConfig> {
    use crate::config::{ErrorStrategy, ImportFormat, TransactionMode};

    let config = ImportConfig {
        schema: args.schema.clone(),
        table: args.table.clone().context("--table is required")?,
        input_file: args.input.clone().context("--input is required")?,
        source_columns: args.source_columns.as_ref().map(|s| s.split(',').map(|s| s.trim().to_string()).collect()),
        target_columns: args.target_columns.as_ref().map(|s| s.split(',').map(|s| s.trim().to_string()).collect()),
        column_mapping: args.column_mapping.as_ref().map(|s| parse_column_mapping(s)).transpose()?,
        column_expressions: None,
        skip_columns: args.skip_columns.as_ref().map(|s| s.split(',').map(|s| s.trim().to_string()).collect()),
        column_types: args.column_types.as_ref().map(|s| parse_column_types(s)).transpose()?,
        format: args.format.as_ref().map(|s| s.parse().map_err(|e: String| anyhow!(e))).transpose()?.unwrap_or(ImportFormat::Csv),
        delimiter: args.delimiter.clone().unwrap_or_else(|| ",".to_string()),
        escape: args.escape.clone(),
        has_header: args.header_override().unwrap_or(true),
        batch_size: args.batch_size.unwrap_or(1000),
        null_value: args.null_value.clone().unwrap_or_default(),
        on_error: args.on_error.as_ref().map(|s| s.parse().map_err(|e: String| anyhow!(e))).transpose()?.unwrap_or(ErrorStrategy::Skip),
        transaction_mode: args.transaction.as_ref().map(|s| s.parse().map_err(|e: String| anyhow!(e))).transpose()?.unwrap_or(TransactionMode::PerBatch),
        show_progress: args.progress_override().unwrap_or(false),
        progress_interval: args.progress_interval.unwrap_or(1_000_000),
        truncate_table: args.truncate,
        pre_sql: args.pre_sql.clone(),
        post_sql: args.post_sql.clone(),
        error_log_table: args.error_log_table.clone(),
        compression: args.compression.as_ref().map(|s| parse_compression_type(s)).transpose()?.unwrap_or(CompressionType::None),
    };

    validate_import_target(&config)?;

    Ok(config)
}

fn parse_column_mapping(s: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for pair in s.split(',') {
        let parts: Vec<_> = pair.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid column mapping format: {}", pair));
        }
        map.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
    }
    Ok(map)
}

fn parse_column_types(s: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut map = std::collections::HashMap::new();
    for pair in s.split(',') {
        let parts: Vec<_> = pair.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid column types format: {}", pair));
        }
        map.insert(parts[0].trim().to_string(), parts[1].trim().to_string());
    }
    Ok(map)
}

fn validate_import_target(config: &ImportConfig) -> Result<()> {
    if config.table.contains('.') {
        return Err(anyhow!(
            "table must not contain schema; use the separate schema field or --schema"
        ));
    }
    if let Some(schema) = &config.schema {
        if schema.contains('.') {
            return Err(anyhow!("schema must be a single schema name"));
        }
    }
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
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<ResolvedExportConfig> {
    if let Some(config_path) = args.config.clone() {
        let cfg = Config::from_file(&config_path)?;
        let resolved_vars = merge_template_vars(cfg.vars, vars_override);
        let database = merge_database_config(cfg.database, &args);
        let export = merge_export_config(
            cfg.export.unwrap_or_else(|| build_export_config_from_args(&args).unwrap()),
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
    } else if (config.db_type == "postgresql" || config.db_type == "greenplum") && config.password.is_empty() {
        if let Ok(pgpassword) = std::env::var("PGPASSWORD") {
            config.password = pgpassword;
        }
    }
    if let Some(fetch) = args.fetch {
        config.fetch_size = fetch;
    }

    config
}

fn merge_export_config(mut config: ExportConfig, args: &ExportArgs) -> Result<ExportConfig> {
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
    if let Some(count_rows) = args.count_rows_override() {
        config.count_rows = count_rows;
    }

    Ok(config)
}

fn merge_logging_config(
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

fn build_database_config_from_args(args: &ExportArgs) -> Result<DatabaseConfig> {
    let db_type = args.db_type.clone().unwrap_or_else(|| "oracle".to_string());
    let password = if let Some(pwd) = &args.password {
        pwd.clone()
    } else if db_type == "postgresql" || db_type == "greenplum" {
        std::env::var("PGPASSWORD").unwrap_or_else(|_| {
            panic!("Password is required. Use --password or set PGPASSWORD environment variable")
        })
    } else {
        required_arg(&args.password, "Password")?
    };

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
        show_progress: args.progress_override().unwrap_or(false),
        include_header: args.header_override().unwrap_or(false),
        buffer_size: args.buffer_size.unwrap_or(1024 * 1024),
        compression,
        progress_interval: args.progress_interval.unwrap_or(1_000_000),
        skip_errors: false,
        count_rows: args.count_rows_override().unwrap_or(false),
    })
}

fn build_database(config: DatabaseConfig) -> Result<Box<dyn Database>> {
    match config.db_type.to_lowercase().as_str() {
        "mysql" => Ok(Box::new(MySqlDatabase::new(config))),
        "oracle" => Ok(Box::new(OracleDatabase::new(config))),
        "postgresql" => Ok(Box::new(PostgreSqlDatabase::new(config))),
        "greenplum" => Ok(Box::new(GreenplumDatabase::new(config))),
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

fn parse_cli_vars(raw_vars: &[String]) -> Result<HashMap<String, String>> {
    let mut vars = HashMap::new();
    for entry in raw_vars {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid --var format '{}', expected key=value", entry))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!("invalid --var format '{}', variable name is empty", entry));
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

fn apply_export_templates(
    mut config: ExportConfig,
    vars: &HashMap<String, String>,
) -> Result<ExportConfig> {
    config.query = resolve_export_query(&config.query, vars)?;
    config.output_file = render_template(&config.output_file, vars, &HashSet::new())?;
    Ok(config)
}

fn resolve_export_query(input: &str, vars: &HashMap<String, String>) -> Result<String> {
    let rendered_input = render_template(input, vars, &HashSet::new())?;
    let query = read_query_or_file(&rendered_input)?;

    if query == rendered_input {
        Ok(query)
    } else {
        render_template(&query, vars, &HashSet::new())
    }
}

fn render_template(
    input: &str,
    vars: &HashMap<String, String>,
    allowed_unresolved: &HashSet<&str>,
) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let end = after_start.find('}').ok_or_else(|| {
            anyhow!("unclosed template variable in '{}'", input)
        })?;
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

#[cfg(test)]
mod tests {
    use super::{
        apply_export_templates, build_import_config_from_args, merge_database_config,
        merge_export_config, parse_cli_vars, render_template, resolve_export_query,
    };
    use crate::cli::{Cli, ExportArgs, ImportArgs};
    use crate::config::{CompressionType, DatabaseConfig, ExportConfig, ExportFormat, LoggingConfig};
    use std::collections::{HashMap, HashSet};

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
            count_rows: false,
            no_count_rows: false,
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
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
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
            count_rows: false,
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
            count_rows: false,
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
            LoggingConfig {
                log_file: None,
                tag: None,
                verbose: true,
            },
            &empty_args(),
            Some(false),
            None,
        );

        assert!(!logging.verbose);
    }

    #[test]
    fn merge_logging_config_allows_global_tag_override() {
        let logging = super::merge_logging_config(
            LoggingConfig {
                log_file: None,
                tag: Some("config-tag".to_string()),
                verbose: false,
            },
            &empty_args(),
            None,
            Some("cli-tag".to_string()),
        );

        assert_eq!(logging.tag.as_deref(), Some("cli-tag"));
    }

    #[test]
    fn cli_exposes_log_tag_override() {
        let cli = <Cli as clap::Parser>::parse_from(["el", "--log-tag", "batch-01", "init", "--list"]);

        assert_eq!(cli.log_tag.as_deref(), Some("batch-01"));
    }

    #[test]
    fn parse_cli_vars_accepts_repeated_key_value_pairs() {
        let vars = parse_cli_vars(&["date=20260329".to_string(), "sync_mode=full".to_string()])
            .expect("vars should parse");

        assert_eq!(vars.get("date").map(String::as_str), Some("20260329"));
        assert_eq!(vars.get("sync_mode").map(String::as_str), Some("full"));
    }

    #[test]
    fn render_template_replaces_known_variables_and_keeps_ext_table() {
        let vars = HashMap::from([
            ("start_date".to_string(), "2026-03-01".to_string()),
            ("datasource".to_string(), "crm".to_string()),
        ]);

        let rendered = render_template(
            "delete from {datasource}.t using {ext_table} where dt >= '{start_date}'",
            &vars,
            &HashSet::from(["ext_table"]),
        )
        .expect("template should render");

        assert_eq!(
            rendered,
            "delete from crm.t using {ext_table} where dt >= '2026-03-01'"
        );
    }

    #[test]
    fn render_template_errors_when_variable_is_missing() {
        let err = render_template("risk/{date}/{datasource}.dat", &HashMap::new(), &HashSet::new())
            .expect_err("missing variable should fail");

        assert!(err.to_string().contains("missing template variable: date"));
    }

    #[test]
    fn apply_export_templates_replaces_query_and_output_file_variables() {
        let config = ExportConfig {
            query: "select * from {schema}.{table} where dt = '{batch_date}'".to_string(),
            output_file: "out/{table}_{batch_date}.csv".to_string(),
            format: ExportFormat::Csv,
            delimiter: ",".to_string(),
            show_progress: false,
            include_header: false,
            buffer_size: 1024,
            compression: CompressionType::None,
            progress_interval: 10,
            skip_errors: false,
            count_rows: false,
        };
        let vars = HashMap::from([
            ("schema".to_string(), "public".to_string()),
            ("table".to_string(), "orders".to_string()),
            ("batch_date".to_string(), "20260329".to_string()),
        ]);

        let rendered = apply_export_templates(config, &vars).expect("export templates should render");

        assert_eq!(
            rendered.query,
            "select * from public.orders where dt = '20260329'"
        );
        assert_eq!(rendered.output_file, "out/orders_20260329.csv");
    }

    #[test]
    fn resolve_export_query_supports_template_in_file_path_and_file_content() {
        let temp_dir = std::env::temp_dir().join(format!(
            "el_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let query_file = temp_dir.join("orders.sql");
        std::fs::write(
            &query_file,
            "select * from {schema}.orders where dt = '{batch_date}'",
        )
        .expect("query file should be written");

        let vars = HashMap::from([
            ("schema".to_string(), "public".to_string()),
            ("table_name".to_string(), "orders".to_string()),
            ("batch_date".to_string(), "20260329".to_string()),
        ]);

        let rendered = resolve_export_query(
            &temp_dir.join("{table_name}.sql").to_string_lossy(),
            &vars,
        )
        .expect("query should resolve");

        assert_eq!(
            rendered,
            "select * from public.orders where dt = '20260329'"
        );

        let _ = std::fs::remove_file(query_file);
        let _ = std::fs::remove_dir(temp_dir);
    }

    #[test]
    fn import_config_rejects_schema_qualified_table_name() {
        let args = ImportArgs {
            config: None,
            db_type: Some("greenplum".to_string()),
            conn: Some("localhost:5432/db".to_string()),
            username: Some("gpadmin".to_string()),
            password: None,
            table: Some("htdw_bak.test_d_risk".to_string()),
            schema: None,
            source_columns: Some("c1,c2".to_string()),
            target_columns: Some("id,name".to_string()),
            column_mapping: None,
            skip_columns: None,
            column_types: None,
            input: Some("risk/data.dat".to_string()),
            format: Some("custom".to_string()),
            delimiter: Some("\u{3}".to_string()),
            escape: None,
            progress: false,
            no_progress: false,
            header: false,
            no_header: true,
            batch_size: None,
            null_value: None,
            on_error: None,
            transaction: None,
            truncate: false,
            pre_sql: None,
            post_sql: None,
            error_log_table: None,
            compression: None,
            log_file: None,
            progress_interval: None,
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
        };

        let err = build_import_config_from_args(&args).expect_err("schema-qualified table should fail");

        assert!(err.to_string().contains("table must not contain schema"));
    }
}
