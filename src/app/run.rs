use crate::cli::{ExportArgs, ImportArgs};
use crate::config::DatabaseConfig;
use crate::db::Database;
use crate::db::greenplum::GreenplumDatabase;
use crate::db::mysql::MySqlDatabase;
use crate::db::oracle::OracleDatabase;
use crate::db::postgresql::PostgreSqlDatabase;
use crate::export::Exporter;
use crate::import::Importer;
use crate::logging::init_tracing;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use tracing::{error, info};

use super::resolve::{
    ResolvedExportConfig, ResolvedImportConfig, compression_type_name, error_strategy_name,
    export_format_name, import_format_name, resolve_export_config, resolve_import_config,
    transaction_mode_name,
};
use super::validate::{validate_resolved_export_config, validate_resolved_import_config};

pub(crate) fn run_export(
    args: ExportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<()> {
    let dry_run = args.dry_run;
    let print_resolved_config = args.print_resolved_config;
    let resolved = resolve_export_config(args, verbose_override, log_tag_override, vars_override)?;
    validate_resolved_export_config(&resolved)?;

    init_tracing(
        resolved.logging.log_file.as_deref(),
        resolved.logging.tag.as_deref(),
        resolved.logging.verbose,
    )?;

    if let Some(config_path) = &resolved.config_path {
        info!(path = %config_path, "config_loaded");
    }

    tracing::debug!(
        db_type = %resolved.database.db_type,
        connection_string = %resolved.database.redacted_connection_string(),
        username = %resolved.database.username,
        fetch_size = resolved.database.fetch_size,
        output = %resolved.export.output_file,
        format = ?resolved.export.format,
        delimiter = ?resolved.export.delimiter,
        progress_interval_secs = resolved.export.progress_interval_secs,
        include_header = resolved.export.include_header,
        buffer_size = resolved.export.buffer_size,
        compression = ?resolved.export.compression,
        "export_config_resolved"
    );
    tracing::debug!(phase = "export_query", sql = %resolved.export.query, "sql_preview");

    if print_resolved_config {
        println!("{}", build_export_resolved_config_text(&resolved));
        return Ok(());
    }

    if dry_run {
        println!("{}", build_export_dry_run_plan(&resolved));
        return Ok(());
    }

    info!(db_type = %resolved.database.db_type, "db_connect_start");
    let mut db = match build_database(resolved.database) {
        Ok(db) => db,
        Err(e) => {
            error!(error = %e, "db_build_failed");
            return Err(e);
        }
    };

    if let Err(e) = db.connect() {
        error!(error = %e, "db_connect_failed");
        return Err(e);
    }
    info!("db_connect_ok");

    info!("export_start");
    let mut exporter = Exporter::new(resolved.export);
    let stats = match exporter.export(db.as_mut()) {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "export_failed");
            return Err(e);
        }
    };

    stats.print_summary();

    Ok(())
}

pub(crate) fn run_import(
    args: ImportArgs,
    verbose_override: Option<bool>,
    log_tag_override: Option<String>,
    vars_override: HashMap<String, String>,
) -> Result<()> {
    let dry_run = args.dry_run;
    let print_resolved_config = args.print_resolved_config;
    let resolved = resolve_import_config(args, verbose_override, log_tag_override, vars_override)?;
    validate_resolved_import_config(&resolved)?;

    init_tracing(
        resolved.logging.log_file.as_deref(),
        resolved.logging.tag.as_deref(),
        resolved.logging.verbose,
    )?;

    if let Some(config_path) = &resolved.config_path {
        info!(path = %config_path, "config_loaded");
    }

    tracing::debug!(database = ?resolved.database, import = ?resolved.import, "import_config_resolved");

    if print_resolved_config {
        println!("{}", build_import_resolved_config_text(&resolved));
        return Ok(());
    }

    if dry_run {
        println!("{}", build_import_dry_run_plan(&resolved));
        return Ok(());
    }

    info!(db_type = %resolved.database.db_type, "db_connect_start");
    let mut db = match build_database(resolved.database) {
        Ok(db) => db,
        Err(e) => {
            error!(error = %e, "db_build_failed");
            return Err(e);
        }
    };

    if let Err(e) = db.connect() {
        error!(error = %e, "db_connect_failed");
        return Err(e);
    }
    info!("db_connect_ok");

    info!("import_start");
    let mut importer = Importer::new(db, resolved.import);
    if let Err(e) = importer.import() {
        error!(error = %e, "import_failed");
        return Err(e);
    }

    Ok(())
}

pub(crate) fn build_import_dry_run_plan(resolved: &ResolvedImportConfig) -> String {
    let config_path = resolved.config_path.as_deref().unwrap_or("<cli>");
    let schema = resolved.import.schema.as_deref().unwrap_or("<none>");
    let source_columns = format_optional_list(resolved.import.source_columns.as_ref());
    let target_columns = format_optional_list(resolved.import.target_columns.as_ref());
    let pre_sql = resolved.import.pre_sql.as_deref().unwrap_or("<none>");
    let post_sql = resolved.import.post_sql.as_deref().unwrap_or("<none>");
    let error_log_table = resolved
        .import
        .error_log_table
        .as_deref()
        .unwrap_or("<none>");
    let gpfdist_host = resolved
        .database
        .gpfdist_host
        .as_deref()
        .unwrap_or("<none>");
    let gpfdist_port = resolved
        .database
        .gpfdist_port
        .map(|port| port.to_string())
        .unwrap_or_else(|| "<none>".to_string());

    format!(
        concat!(
            "mode: import\n",
            "dry_run: true\n",
            "config_path: {config_path}\n",
            "db_type: {db_type}\n",
            "connection: {connection}\n",
            "username: {username}\n",
            "schema: {schema}\n",
            "table: {table}\n",
            "input_file: {input_file}\n",
            "format: {format}\n",
            "delimiter: {delimiter}\n",
            "has_header: {has_header}\n",
            "batch_size: {batch_size}\n",
            "compression: {compression}\n",
            "on_error: {on_error}\n",
            "transaction_mode: {transaction_mode}\n",
            "truncate_table: {truncate_table}\n",
            "show_progress: {show_progress}\n",
            "progress_interval_secs: {progress_interval_secs}\n",
            "source_columns: {source_columns}\n",
            "target_columns: {target_columns}\n",
            "pre_sql: {pre_sql}\n",
            "post_sql: {post_sql}\n",
            "error_log_table: {error_log_table}\n",
            "gpfdist_host: {gpfdist_host}\n",
            "gpfdist_port: {gpfdist_port}"
        ),
        config_path = config_path,
        db_type = resolved.database.db_type,
        connection = resolved.database.redacted_connection_string(),
        username = resolved.database.username,
        schema = schema,
        table = resolved.import.table,
        input_file = resolved.import.input_file,
        format = import_format_name(&resolved.import.format),
        delimiter = resolved.import.delimiter,
        has_header = resolved.import.has_header,
        batch_size = resolved.import.batch_size,
        compression = compression_type_name(&resolved.import.compression),
        on_error = error_strategy_name(&resolved.import.on_error),
        transaction_mode = transaction_mode_name(&resolved.import.transaction_mode),
        truncate_table = resolved.import.truncate_table,
        show_progress = resolved.import.show_progress,
        progress_interval_secs = resolved.import.progress_interval_secs,
        source_columns = source_columns,
        target_columns = target_columns,
        pre_sql = pre_sql,
        post_sql = post_sql,
        error_log_table = error_log_table,
        gpfdist_host = gpfdist_host,
        gpfdist_port = gpfdist_port,
    )
}

pub(crate) fn build_import_resolved_config_text(resolved: &ResolvedImportConfig) -> String {
    let config_path = resolved.config_path.as_deref().unwrap_or("<cli>");
    let password = if resolved.database.password.is_empty() {
        ""
    } else {
        "***"
    };

    format!(
        concat!(
            "mode = {mode}\n",
            "config_path = {config_path}\n\n",
            "[database]\n",
            "db_type = {db_type}\n",
            "connection_string = {connection_string}\n",
            "username = {username}\n",
            "password = {password}\n",
            "fetch_size = {fetch_size}\n",
            "gpfdist_host = {gpfdist_host}\n",
            "gpfdist_port = {gpfdist_port}\n\n",
            "[logging]\n",
            "log_file = {log_file}\n",
            "tag = {tag}\n",
            "verbose = {verbose}\n\n",
            "[import]\n",
            "schema = {schema}\n",
            "table = {table}\n",
            "input_file = {input_file}\n",
            "format = {format}\n",
            "delimiter = {delimiter}\n",
            "has_header = {has_header}\n",
            "batch_size = {batch_size}\n",
            "null_value = {null_value}\n",
            "on_error = {on_error}\n",
            "transaction_mode = {transaction_mode}\n",
            "show_progress = {show_progress}\n",
            "progress_interval_secs = {progress_interval_secs}\n",
            "truncate_table = {truncate_table}\n",
            "source_columns = {source_columns}\n",
            "target_columns = {target_columns}\n",
            "pre_sql = {pre_sql}\n",
            "post_sql = {post_sql}\n",
            "error_log_table = {error_log_table}\n",
            "compression = {compression}"
        ),
        mode = quote_string("import"),
        config_path = quote_string(config_path),
        db_type = quote_string(&resolved.database.db_type),
        connection_string = quote_string(&resolved.database.redacted_connection_string()),
        username = quote_string(&resolved.database.username),
        password = quote_string(password),
        fetch_size = resolved.database.fetch_size,
        gpfdist_host = format_optional_string(resolved.database.gpfdist_host.as_deref()),
        gpfdist_port = format_optional_u16(resolved.database.gpfdist_port),
        log_file = format_optional_string(resolved.logging.log_file.as_deref()),
        tag = format_optional_string(resolved.logging.tag.as_deref()),
        verbose = resolved.logging.verbose,
        schema = format_optional_string(resolved.import.schema.as_deref()),
        table = quote_string(&resolved.import.table),
        input_file = quote_string(&resolved.import.input_file),
        format = quote_string(import_format_name(&resolved.import.format)),
        delimiter = quote_string(&resolved.import.delimiter),
        has_header = resolved.import.has_header,
        batch_size = resolved.import.batch_size,
        null_value = quote_string(&resolved.import.null_value),
        on_error = quote_string(error_strategy_name(&resolved.import.on_error)),
        transaction_mode = quote_string(transaction_mode_name(&resolved.import.transaction_mode)),
        show_progress = resolved.import.show_progress,
        progress_interval_secs = resolved.import.progress_interval_secs,
        truncate_table = resolved.import.truncate_table,
        source_columns = format_optional_list_for_config(resolved.import.source_columns.as_ref()),
        target_columns = format_optional_list_for_config(resolved.import.target_columns.as_ref()),
        pre_sql = format_optional_string(resolved.import.pre_sql.as_deref()),
        post_sql = format_optional_string(resolved.import.post_sql.as_deref()),
        error_log_table = format_optional_string(resolved.import.error_log_table.as_deref()),
        compression = quote_string(compression_type_name(&resolved.import.compression)),
    )
}

pub(crate) fn build_export_dry_run_plan(resolved: &ResolvedExportConfig) -> String {
    let config_path = resolved.config_path.as_deref().unwrap_or("<cli>");

    format!(
        concat!(
            "mode: export\n",
            "dry_run: true\n",
            "config_path: {config_path}\n",
            "db_type: {db_type}\n",
            "connection: {connection}\n",
            "username: {username}\n",
            "fetch_size: {fetch_size}\n",
            "query: {query}\n",
            "output_file: {output_file}\n",
            "format: {format}\n",
            "delimiter: {delimiter}\n",
            "include_header: {include_header}\n",
            "compression: {compression}\n",
            "buffer_size: {buffer_size}\n",
            "progress_interval_secs: {progress_interval_secs}\n",
            "count_rows: {count_rows}"
        ),
        config_path = config_path,
        db_type = resolved.database.db_type,
        connection = resolved.database.redacted_connection_string(),
        username = resolved.database.username,
        fetch_size = resolved.database.fetch_size,
        query = resolved.export.query,
        output_file = resolved.export.output_file,
        format = export_format_name(&resolved.export.format),
        delimiter = resolved.export.delimiter,
        include_header = resolved.export.include_header,
        compression = compression_type_name(&resolved.export.compression),
        buffer_size = resolved.export.buffer_size,
        progress_interval_secs = resolved.export.progress_interval_secs,
        count_rows = resolved.export.count_rows,
    )
}

pub(crate) fn build_export_resolved_config_text(resolved: &ResolvedExportConfig) -> String {
    let config_path = resolved.config_path.as_deref().unwrap_or("<cli>");
    let password = if resolved.database.password.is_empty() {
        ""
    } else {
        "***"
    };

    format!(
        concat!(
            "mode = {mode}\n",
            "config_path = {config_path}\n\n",
            "[database]\n",
            "db_type = {db_type}\n",
            "connection_string = {connection_string}\n",
            "username = {username}\n",
            "password = {password}\n",
            "fetch_size = {fetch_size}\n\n",
            "[logging]\n",
            "log_file = {log_file}\n",
            "tag = {tag}\n",
            "verbose = {verbose}\n\n",
            "[export]\n",
            "query = {query}\n",
            "output_file = {output_file}\n",
            "format = {format}\n",
            "delimiter = {delimiter}\n",
            "include_header = {include_header}\n",
            "compression = {compression}\n",
            "buffer_size = {buffer_size}\n",
            "progress_interval_secs = {progress_interval_secs}\n",
            "count_rows = {count_rows}"
        ),
        mode = quote_string("export"),
        config_path = quote_string(config_path),
        db_type = quote_string(&resolved.database.db_type),
        connection_string = quote_string(&resolved.database.redacted_connection_string()),
        username = quote_string(&resolved.database.username),
        password = quote_string(password),
        fetch_size = resolved.database.fetch_size,
        log_file = format_optional_string(resolved.logging.log_file.as_deref()),
        tag = format_optional_string(resolved.logging.tag.as_deref()),
        verbose = resolved.logging.verbose,
        query = quote_string(&resolved.export.query),
        output_file = quote_string(&resolved.export.output_file),
        format = quote_string(export_format_name(&resolved.export.format)),
        delimiter = quote_string(&resolved.export.delimiter),
        include_header = resolved.export.include_header,
        compression = quote_string(compression_type_name(&resolved.export.compression)),
        buffer_size = resolved.export.buffer_size,
        progress_interval_secs = resolved.export.progress_interval_secs,
        count_rows = resolved.export.count_rows,
    )
}

pub(crate) fn build_database(config: DatabaseConfig) -> Result<Box<dyn Database>> {
    match config.db_type.to_lowercase().as_str() {
        "mysql" => Ok(Box::new(MySqlDatabase::new(config))),
        "oracle" => Ok(Box::new(OracleDatabase::new(config))),
        "postgresql" => Ok(Box::new(PostgreSqlDatabase::new(config))),
        "greenplum" => Ok(Box::new(GreenplumDatabase::new(config))),
        other => Err(anyhow!("Unsupported database type: {other}")),
    }
}

fn quote_string(value: &str) -> String {
    format!("{value:?}")
}

fn format_optional_string(value: Option<&str>) -> String {
    match value {
        Some(value) => quote_string(value),
        None => "null".to_string(),
    }
}

fn format_optional_list(value: Option<&Vec<String>>) -> String {
    match value {
        Some(values) => values.join(","),
        None => "<none>".to_string(),
    }
}

fn format_optional_list_for_config(value: Option<&Vec<String>>) -> String {
    match value {
        Some(values) => quote_string(&format!("[{}]", values.join(","))),
        None => "null".to_string(),
    }
}

fn format_optional_u16(value: Option<u16>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "null".to_string(),
    }
}
