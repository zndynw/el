use crate::config::DatabaseConfig;
use crate::db::pgpass::{PgPassPlatform, PgPassTarget, load_password, select_path};
use anyhow::{Context, Result, anyhow};
use postgres::{Client, Config, NoTls, config::Host};
use std::env;
use std::ffi::OsString;
use tracing::warn;

#[derive(Clone, Copy, Debug)]
pub(crate) enum DatabaseKind {
    PostgreSql,
    Greenplum,
}

impl DatabaseKind {
    fn display_name(self) -> &'static str {
        match self {
            Self::PostgreSql => "PostgreSQL",
            Self::Greenplum => "Greenplum",
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct CredentialSources {
    #[cfg(test)]
    pub(crate) pgpassword: Option<Vec<u8>>,
    pub(crate) pgpassfile: Option<OsString>,
    pub(crate) home: Option<OsString>,
    pub(crate) appdata: Option<OsString>,
}

pub(crate) fn connect_from_process(
    database: &DatabaseConfig,
    kind: DatabaseKind,
) -> Result<Client> {
    let configs = build_configs_from_process(database, kind)?;
    let mut last_error = None;

    for config in configs {
        match config.connect(NoTls) {
            Ok(client) => return Ok(client),
            Err(error) => last_error = Some(error),
        }
    }

    match last_error {
        Some(error) => Err(error.into()),
        None => Err(anyhow!(
            "No {} connection targets configured",
            kind.display_name()
        )),
    }
}

#[cfg(test)]
fn build_config(
    database: &DatabaseConfig,
    kind: DatabaseKind,
    sources: &CredentialSources,
) -> Result<Config> {
    take_single_config(build_configs(database, kind, sources)?)
}

#[cfg(test)]
fn build_configs(
    database: &DatabaseConfig,
    kind: DatabaseKind,
    sources: &CredentialSources,
) -> Result<Vec<Config>> {
    let configs = parse_connection_configs(database, kind)?;
    apply_passwords(
        configs,
        database,
        || sources.pgpassword.clone(),
        || sources.clone(),
    )
}

fn build_configs_from_process(
    database: &DatabaseConfig,
    kind: DatabaseKind,
) -> Result<Vec<Config>> {
    let configs = parse_connection_configs(database, kind)?;
    apply_passwords(
        configs,
        database,
        || env::var("PGPASSWORD").ok().map(String::into_bytes),
        || CredentialSources {
            #[cfg(test)]
            pgpassword: None,
            pgpassfile: env::var_os("PGPASSFILE"),
            home: env::var_os("HOME"),
            appdata: env::var_os("APPDATA"),
        },
    )
}

fn apply_passwords<P, F>(
    mut configs: Vec<Config>,
    database: &DatabaseConfig,
    load_pgpassword: P,
    load_pgpass_sources: F,
) -> Result<Vec<Config>>
where
    P: FnOnce() -> Option<Vec<u8>>,
    F: FnOnce() -> CredentialSources,
{
    for config in &mut configs {
        if config.get_user().is_none() {
            config.user(&database.username);
        }
    }

    if !database.password.is_empty() {
        for config in &mut configs {
            config.password(database.password.as_bytes());
        }
        return Ok(configs);
    }

    let unresolved = configs
        .iter()
        .enumerate()
        .filter_map(|(index, config)| config.get_password().is_none().then_some(index))
        .collect::<Vec<_>>();
    if unresolved.is_empty() {
        return Ok(configs);
    }

    if let Some(password) = load_pgpassword() {
        for index in unresolved {
            configs[index].password(&password);
        }
        return Ok(configs);
    }

    let sources = load_pgpass_sources();
    for index in unresolved {
        let config = &mut configs[index];
        let password = lookup_pgpass_password(config, &sources);
        if config.get_hosts().len().max(config.get_hostaddrs().len()) > 1 {
            if password.is_some() {
                return Err(anyhow!(
                    "pgpass fallback cannot safely apply one password to multiple PostgreSQL hosts"
                ));
            }
            continue;
        }
        if let Some(password) = password {
            config.password(password);
        }
    }

    Ok(configs)
}

#[cfg(test)]
fn take_single_config(mut configs: Vec<Config>) -> Result<Config> {
    if configs.len() != 1 {
        return Err(anyhow!(
            "Expected one PostgreSQL connection target, found {}",
            configs.len()
        ));
    }
    Ok(configs.remove(0))
}

fn parse_connection_configs(database: &DatabaseConfig, kind: DatabaseKind) -> Result<Vec<Config>> {
    if database.connection_string.starts_with("postgresql://")
        || database.connection_string.starts_with("postgres://")
    {
        return split_connection_targets(&database.connection_string)?
            .into_iter()
            .map(|connection_string| parse_url_config(&connection_string, kind))
            .collect();
    }

    Ok(vec![parse_compact_config(database, kind)?])
}

fn parse_url_config(connection_string: &str, kind: DatabaseKind) -> Result<Config> {
    let mut config = connection_string
        .parse::<Config>()
        .with_context(|| format!("Invalid {} connection string", kind.display_name()))?;
    if config.get_hosts().is_empty() && config.get_hostaddrs().is_empty() {
        config.host("localhost");
    }
    Ok(config)
}

fn parse_compact_config(database: &DatabaseConfig, kind: DatabaseKind) -> Result<Config> {
    let target = parse_compact_target(&database.connection_string, kind)?;
    let mut config = Config::new();
    config
        .host(&target.host)
        .port(target.port)
        .dbname(&target.database)
        .user(&database.username);
    Ok(config)
}

fn split_connection_targets(connection_string: &str) -> Result<Vec<String>> {
    let Some(scheme_end) = connection_string.find("://") else {
        return Ok(vec![connection_string.to_string()]);
    };
    let authority_start = scheme_end + 3;
    let authority_end = connection_string[authority_start..]
        .find(['/', '?', '#'])
        .map(|offset| authority_start + offset)
        .unwrap_or(connection_string.len());
    let authority = &connection_string[authority_start..authority_end];
    let hosts_start = authority
        .rfind('@')
        .map(|offset| authority_start + offset + 1)
        .unwrap_or(authority_start);
    let hosts = &connection_string[hosts_start..authority_end];
    let host_parts = split_host_list(hosts);
    let fragment_start = connection_string
        .find('#')
        .unwrap_or(connection_string.len());
    let query_start = connection_string[..fragment_start].find('?');
    let path_end = query_start.unwrap_or(fragment_start);
    let query_parts = query_start
        .map(|start| {
            connection_string[start + 1..fragment_start]
                .split('&')
                .collect()
        })
        .unwrap_or_else(Vec::new);

    let query_host_count = query_list_len(&query_parts, "host");
    let query_hostaddr_count = query_list_len(&query_parts, "hostaddr");
    let target_count = host_parts
        .len()
        .max(query_host_count)
        .max(query_hostaddr_count);

    if target_count <= 1 {
        return Ok(vec![connection_string.to_string()]);
    }

    for (name, count) in [
        ("authority host", host_parts.len()),
        ("host", query_host_count),
        ("hostaddr", query_hostaddr_count),
        ("port", query_list_len(&query_parts, "port")),
    ] {
        if count > 1 && count != target_count {
            return Err(anyhow!(
                "PostgreSQL {name} list has {count} entries but expected {target_count}"
            ));
        }
    }

    let mut targets = Vec::with_capacity(target_count);
    for index in 0..target_count {
        let authority_host = if host_parts.len() == target_count {
            host_parts[index]
        } else {
            hosts
        };
        let mut target = format!(
            "{}{}{}",
            &connection_string[..hosts_start],
            authority_host,
            &connection_string[authority_end..path_end]
        );
        if query_start.is_some() {
            target.push('?');
            target.push_str(
                &query_parts
                    .iter()
                    .map(|part| select_query_list_entry(part, index, target_count))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        target.push_str(&connection_string[fragment_start..]);
        targets.push(target);
    }

    Ok(targets)
}

fn query_list_len(query_parts: &[&str], name: &str) -> usize {
    query_parts
        .iter()
        .find_map(|part| {
            let (key, value) = part.split_once('=')?;
            (key == name).then(|| value.split(',').count())
        })
        .unwrap_or(0)
}

fn select_query_list_entry(part: &str, index: usize, target_count: usize) -> String {
    let Some((key, value)) = part.split_once('=') else {
        return part.to_string();
    };
    if !matches!(key, "host" | "hostaddr" | "port") {
        return part.to_string();
    }

    let values = value.split(',').collect::<Vec<_>>();
    if values.len() == target_count {
        format!("{key}={}", values[index])
    } else {
        part.to_string()
    }
}

fn split_host_list(hosts: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut bracket_depth: usize = 0;

    for (index, ch) in hosts.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' if bracket_depth == 0 => {
                result.push(&hosts[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(&hosts[start..]);
    result
}

struct ConnectionTarget {
    host: String,
    port: u16,
    database: String,
}

fn parse_compact_target(value: &str, kind: DatabaseKind) -> Result<ConnectionTarget> {
    let name = kind.display_name();
    let (host_port, database) = value.rsplit_once('/').ok_or_else(|| {
        anyhow!("{name} connection string must be host:port/database or host/database")
    })?;

    let database = database.trim();
    if database.is_empty() {
        return Err(anyhow!("{name} connection string must include database"));
    }

    let (host, port) = if let Some((host, port)) = host_port.rsplit_once(':') {
        let host = host.trim();
        if host.is_empty() {
            return Err(anyhow!("{name} connection string must include host"));
        }
        let port = port
            .trim()
            .parse::<u16>()
            .with_context(|| format!("Invalid {name} port"))?;
        (host.to_string(), port)
    } else {
        let host = host_port.trim();
        if host.is_empty() {
            return Err(anyhow!("{name} connection string must include host"));
        }
        (host.to_string(), 5432)
    };

    Ok(ConnectionTarget {
        host,
        port,
        database: database.to_string(),
    })
}

#[cfg(test)]
fn resolve_password<F>(
    explicit: Option<&[u8]>,
    url: Option<&[u8]>,
    pgpassword: Option<&[u8]>,
    lookup_pgpass: F,
) -> Option<Vec<u8>>
where
    F: FnOnce() -> Option<Vec<u8>>,
{
    explicit
        .or(url)
        .or(pgpassword)
        .map(<[u8]>::to_vec)
        .or_else(lookup_pgpass)
}

fn lookup_pgpass_password(config: &Config, sources: &CredentialSources) -> Option<Vec<u8>> {
    let path = select_path(
        current_platform(),
        sources.pgpassfile.as_deref(),
        sources.home.as_deref(),
        sources.appdata.as_deref(),
    )?;
    let target = pgpass_target(config)?;

    match load_password(&path, &target.as_borrowed()) {
        Ok(password) => password,
        Err(error) => {
            warn!(path = %path.display(), reason = %error, "pgpass_ignored");
            None
        }
    }
}

struct OwnedPgPassTarget {
    host: Vec<u8>,
    port: u16,
    database: Vec<u8>,
    user: Vec<u8>,
}

impl OwnedPgPassTarget {
    fn as_borrowed(&self) -> PgPassTarget<'_> {
        PgPassTarget {
            host: &self.host,
            port: self.port,
            database: &self.database,
            user: &self.user,
        }
    }
}

fn pgpass_target(config: &Config) -> Option<OwnedPgPassTarget> {
    let user = config.get_user()?.as_bytes().to_vec();
    let database = config
        .get_dbname()
        .map(str::as_bytes)
        .unwrap_or(&user)
        .to_vec();
    let host = match config.get_hosts().first() {
        Some(Host::Tcp(host)) => host.as_bytes().to_vec(),
        #[cfg(unix)]
        Some(Host::Unix(_)) => b"localhost".to_vec(),
        None => config
            .get_hostaddrs()
            .first()
            .map(ToString::to_string)
            .map(String::into_bytes)
            .unwrap_or_else(|| b"localhost".to_vec()),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);

    Some(OwnedPgPassTarget {
        host,
        port,
        database,
        user,
    })
}

fn current_platform() -> PgPassPlatform {
    #[cfg(windows)]
    {
        PgPassPlatform::Windows
    }

    #[cfg(not(windows))]
    {
        PgPassPlatform::Unix
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CredentialSources, DatabaseKind, apply_passwords, build_config, build_configs,
        parse_connection_configs, resolve_password,
    };
    use crate::config::DatabaseConfig;
    use postgres::config::Host;
    use std::cell::Cell;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn database_config(connection_string: &str, password: &str) -> DatabaseConfig {
        DatabaseConfig {
            db_type: "postgresql".to_string(),
            connection_string: connection_string.to_string(),
            username: "alice".to_string(),
            password: password.to_string(),
            fetch_size: 1000,
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
        }
    }

    fn temp_path(test_name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "el-postgres-config-{}-{test_name}-{nonce}",
            std::process::id()
        ))
    }

    fn write_pgpass(path: &Path, contents: &[u8]) {
        fs::write(path, contents).expect("test pgpass should be written");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("test pgpass permissions should be set");
        }
    }

    #[test]
    fn explicit_password_overrides_url_and_environment() {
        let database = database_config("postgresql://alice:url@db:5432/app", "explicit");
        let sources = CredentialSources {
            pgpassword: Some(b"environment".to_vec()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();

        assert_eq!(config.get_password(), Some(b"explicit".as_slice()));
    }

    #[test]
    fn process_sources_keep_explicit_password_at_highest_priority() {
        let database = database_config("db:5432/app", "explicit");

        let configs = super::build_configs_from_process(&database, DatabaseKind::PostgreSql)
            .expect("explicit password should avoid lower-priority file lookup");

        assert_eq!(configs[0].get_password(), Some(b"explicit".as_slice()));
    }

    #[test]
    fn url_password_overrides_pgpassword() {
        let database = database_config("postgresql://alice:url@db:5432/app", "");
        let sources = CredentialSources {
            pgpassword: Some(b"environment".to_vec()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();

        assert_eq!(config.get_password(), Some(b"url".as_slice()));
    }

    #[test]
    fn pgpassword_overrides_pgpass_file() {
        let path = temp_path("environment-priority");
        write_pgpass(&path, b"db:5432:app:alice:file-secret\n");
        let database = database_config("db:5432/app", "");
        let sources = CredentialSources {
            pgpassword: Some(b"environment".to_vec()),
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_password(), Some(b"environment".as_slice()));
    }

    #[test]
    fn matching_pgpass_file_supplies_password() {
        let path = temp_path("file-fallback");
        write_pgpass(&path, b"db:5432:app:alice:file-secret\n");
        let database = database_config("db:5432/app", "");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_password(), Some(b"file-secret".as_slice()));
    }

    #[test]
    fn missing_credentials_leave_password_unset() {
        let database = database_config("db:5432/app", "");

        let config = build_config(
            &database,
            DatabaseKind::PostgreSql,
            &CredentialSources::default(),
        )
        .unwrap();

        assert_eq!(config.get_password(), None);
    }

    #[test]
    fn compact_target_uses_configured_and_default_ports() {
        let configured = build_config(
            &database_config("db:6432/app", ""),
            DatabaseKind::PostgreSql,
            &CredentialSources::default(),
        )
        .unwrap();
        let defaulted = build_config(
            &database_config("db/app", ""),
            DatabaseKind::PostgreSql,
            &CredentialSources::default(),
        )
        .unwrap();

        assert_eq!(configured.get_hosts(), &[Host::Tcp("db".to_string())]);
        assert_eq!(configured.get_ports(), &[6432]);
        assert_eq!(configured.get_dbname(), Some("app"));
        assert_eq!(configured.get_user(), Some("alice"));
        assert_eq!(defaulted.get_ports(), &[5432]);
    }

    #[test]
    fn url_fields_are_used_for_pgpass_matching() {
        let path = temp_path("url-fields");
        write_pgpass(&path, b"url-db:6543:url_app:url_user:file-secret\n");
        let database = database_config("postgresql://url_user@url-db:6543/url_app", "");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_password(), Some(b"file-secret".as_slice()));
    }

    #[test]
    fn url_without_user_uses_database_config_username() {
        let database = database_config("postgresql://db:5432/app", "");

        let config = build_config(
            &database,
            DatabaseKind::PostgreSql,
            &CredentialSources::default(),
        )
        .unwrap();

        assert_eq!(config.get_user(), Some("alice"));
    }

    #[test]
    fn missing_url_database_matches_username() {
        let path = temp_path("database-default");
        write_pgpass(&path, b"db:5432:alice:alice:file-secret\n");
        let database = database_config("postgresql://alice@db:5432", "");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_password(), Some(b"file-secret".as_slice()));
    }

    #[test]
    fn hostless_url_uses_localhost_for_pgpass_matching() {
        let path = temp_path("hostless-url");
        write_pgpass(&path, b"localhost:5432:app:alice:file-secret\n");
        let database = database_config("postgresql:///app", "");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_hosts(), &[Host::Tcp("localhost".to_string())]);
        assert_eq!(config.get_password(), Some(b"file-secret".as_slice()));
    }

    #[test]
    fn hostaddr_only_url_uses_hostaddr_for_pgpass_matching() {
        let path = temp_path("hostaddr-only-url");
        write_pgpass(&path, b"127.0.0.1:5432:app:alice:file-secret\n");
        let database = database_config("postgresql:///app?hostaddr=127.0.0.1", "");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_password(), Some(b"file-secret".as_slice()));
    }

    #[test]
    fn multi_host_url_uses_host_specific_pgpass_passwords() {
        let path = temp_path("multi-host");
        write_pgpass(
            &path,
            b"host-one:5432:app:alice:first-secret\nhost-two:6432:app:alice:second-secret\n",
        );
        let database = database_config(
            "postgresql://alice@host-one:5432,host-two:6432/app?application_name=el",
            "",
        );
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let configs = build_configs(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].get_hosts(), &[Host::Tcp("host-one".to_string())]);
        assert_eq!(configs[0].get_ports(), &[5432]);
        assert_eq!(configs[0].get_password(), Some(b"first-secret".as_slice()));
        assert_eq!(configs[1].get_hosts(), &[Host::Tcp("host-two".to_string())]);
        assert_eq!(configs[1].get_ports(), &[6432]);
        assert_eq!(configs[1].get_password(), Some(b"second-secret".as_slice()));
        assert_eq!(configs[0].get_application_name(), Some("el"));
        assert_eq!(configs[1].get_application_name(), Some("el"));
    }

    #[test]
    fn authority_hosts_pair_with_query_hostaddrs() {
        let path = temp_path("authority-hostaddr-pairs");
        write_pgpass(
            &path,
            b"host-one:5432:app:alice:first-secret\nhost-two:5432:app:alice:second-secret\n",
        );
        let database = database_config(
            "postgresql://alice@host-one,host-two/app?hostaddr=10.0.0.1,10.0.0.2",
            "",
        );
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let configs = build_configs(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].get_hosts(), &[Host::Tcp("host-one".to_string())]);
        assert_eq!(configs[0].get_hostaddrs()[0].to_string(), "10.0.0.1");
        assert_eq!(configs[0].get_password(), Some(b"first-secret".as_slice()));
        assert_eq!(configs[1].get_hosts(), &[Host::Tcp("host-two".to_string())]);
        assert_eq!(configs[1].get_hostaddrs()[0].to_string(), "10.0.0.2");
        assert_eq!(configs[1].get_password(), Some(b"second-secret".as_slice()));
    }

    #[test]
    fn query_host_list_uses_host_specific_pgpass_passwords() {
        let path = temp_path("query-host-list");
        write_pgpass(
            &path,
            b"host-one:5432:app:alice:first-secret\nhost-two:6432:app:alice:second-secret\n",
        );
        let database = database_config(
            "postgresql:///app?user=alice&host=host-one,host-two&port=5432,6432",
            "",
        );
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let configs = build_configs(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].get_hosts(), &[Host::Tcp("host-one".to_string())]);
        assert_eq!(configs[0].get_ports(), &[5432]);
        assert_eq!(configs[0].get_password(), Some(b"first-secret".as_slice()));
        assert_eq!(configs[1].get_hosts(), &[Host::Tcp("host-two".to_string())]);
        assert_eq!(configs[1].get_ports(), &[6432]);
        assert_eq!(configs[1].get_password(), Some(b"second-secret".as_slice()));
    }

    #[test]
    fn query_host_list_without_credentials_is_left_for_the_driver() {
        let database = database_config(
            "postgresql:///app?user=alice&host=host-one,host-two&port=5432,6432",
            "",
        );

        let configs = build_configs(
            &database,
            DatabaseKind::PostgreSql,
            &CredentialSources::default(),
        )
        .unwrap();

        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].get_password(), None);
        assert_eq!(configs[1].get_password(), None);
    }

    #[test]
    fn greenplum_validation_uses_greenplum_name() {
        let error = build_config(
            &database_config("missing-database", ""),
            DatabaseKind::Greenplum,
            &CredentialSources::default(),
        )
        .expect_err("target should be invalid");

        assert!(error.to_string().contains("Greenplum"));
    }

    #[test]
    fn higher_priority_password_skips_pgpass_lookup() {
        let called = Cell::new(false);

        let password = resolve_password(
            Some(b"explicit"),
            Some(b"url"),
            Some(b"environment"),
            || {
                called.set(true);
                Some(b"file".to_vec())
            },
        );

        assert_eq!(password, Some(b"explicit".to_vec()));
        assert!(!called.get());
    }

    #[test]
    fn explicit_password_skips_all_process_source_loaders() {
        let database = database_config("db:5432/app", "explicit");
        let configs = parse_connection_configs(&database, DatabaseKind::PostgreSql).unwrap();
        let pgpassword_read = Cell::new(false);
        let pgpass_paths_read = Cell::new(false);

        let configs = apply_passwords(
            configs,
            &database,
            || {
                pgpassword_read.set(true);
                None
            },
            || {
                pgpass_paths_read.set(true);
                CredentialSources::default()
            },
        )
        .unwrap();

        assert_eq!(configs[0].get_password(), Some(b"explicit".as_slice()));
        assert!(!pgpassword_read.get());
        assert!(!pgpass_paths_read.get());
    }

    #[test]
    fn url_password_skips_all_process_source_loaders() {
        let database = database_config("postgresql://alice:url@db:5432/app", "");
        let configs = parse_connection_configs(&database, DatabaseKind::PostgreSql).unwrap();
        let pgpassword_read = Cell::new(false);
        let pgpass_paths_read = Cell::new(false);

        let configs = apply_passwords(
            configs,
            &database,
            || {
                pgpassword_read.set(true);
                None
            },
            || {
                pgpass_paths_read.set(true);
                CredentialSources::default()
            },
        )
        .unwrap();

        assert_eq!(configs[0].get_password(), Some(b"url".as_slice()));
        assert!(!pgpassword_read.get());
        assert!(!pgpass_paths_read.get());
    }

    #[test]
    fn pgpassword_skips_pgpass_path_loader() {
        let database = database_config("db:5432/app", "");
        let configs = parse_connection_configs(&database, DatabaseKind::PostgreSql).unwrap();
        let pgpass_paths_read = Cell::new(false);

        let configs = apply_passwords(
            configs,
            &database,
            || Some(b"environment".to_vec()),
            || {
                pgpass_paths_read.set(true);
                CredentialSources::default()
            },
        )
        .unwrap();

        assert_eq!(configs[0].get_password(), Some(b"environment".as_slice()));
        assert!(!pgpass_paths_read.get());
    }

    #[test]
    fn pgpassfile_path_is_stored_without_unicode_conversion() {
        let path = OsString::from("custom.pass");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone()),
            ..Default::default()
        };

        assert_eq!(sources.pgpassfile, Some(path));
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_uses_localhost_for_pgpass_matching() {
        let path = temp_path("unix-socket");
        write_pgpass(&path, b"localhost:5432:app:alice:file-secret\n");
        let database = database_config("postgresql://alice@%2Fvar%2Frun%2Fpostgresql/app", "");
        let sources = CredentialSources {
            pgpassfile: Some(path.clone().into_os_string()),
            ..Default::default()
        };

        let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
        fs::remove_file(path).expect("test pgpass should be removed");

        assert_eq!(config.get_password(), Some(b"file-secret".as_slice()));
    }
}
