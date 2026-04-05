use crate::config::{ImportConfig, ImportFormat};
use anyhow::{Result, anyhow};

use super::resolve::{ResolvedExportConfig, ResolvedImportConfig, import_format_name};

pub(crate) fn validate_resolved_import_config(resolved: &ResolvedImportConfig) -> Result<()> {
    validate_supported_database_type(&resolved.database.db_type)?;
    validate_import_target(&resolved.import)?;

    if !resolved.import.has_header && resolved.import.source_columns.is_none() {
        return Err(anyhow!(
            "source_columns must be specified when has_header is false"
        ));
    }

    if resolved.database.db_type.eq_ignore_ascii_case("greenplum") {
        if resolved.database.gpfdist_host.is_none() {
            return Err(anyhow!("gpfdist_host not configured"));
        }
        if resolved.database.gpfdist_port.is_none() {
            return Err(anyhow!("gpfdist_port not configured"));
        }
        match resolved.import.format {
            ImportFormat::Tsv | ImportFormat::Custom if resolved.import.has_header => {
                return Err(anyhow!(
                    "Greenplum direct import does not support has_header=true with {} format",
                    import_format_name(&resolved.import.format)
                ));
            }
            _ => {}
        }
    }

    if matches!(resolved.import.format, ImportFormat::Custom)
        && resolved.import.delimiter.is_empty()
    {
        return Err(anyhow!("custom import format requires a delimiter"));
    }

    Ok(())
}

pub(crate) fn validate_resolved_export_config(resolved: &ResolvedExportConfig) -> Result<()> {
    validate_supported_database_type(&resolved.database.db_type)
}

pub(crate) fn validate_import_target(config: &ImportConfig) -> Result<()> {
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

pub(crate) fn validate_supported_database_type(db_type: &str) -> Result<()> {
    match db_type.to_lowercase().as_str() {
        "mysql" | "oracle" | "postgresql" | "greenplum" => Ok(()),
        other => Err(anyhow!("unsupported database type: {other}")),
    }
}
