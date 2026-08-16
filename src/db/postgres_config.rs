use crate::config::DatabaseConfig;
use crate::db::pgpass::{PgPassPlatform, PgPassTarget, load_password, select_path};
use anyhow::{Context, Result, anyhow};
use postgres::{Config, config::Host};
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

#[derive(Default)]
pub(crate) struct CredentialSources {
    pub(crate) pgpassword: Option<Vec<u8>>,
    pub(crate) pgpassfile: Option<OsString>,
    pub(crate) home: Option<OsString>,
    pub(crate) appdata: Option<OsString>,
}

impl CredentialSources {
    fn from_process() -> Self {
        Self {
            pgpassword: env::var("PGPASSWORD").ok().map(String::into_bytes),
            pgpassfile: env::var_os("PGPASSFILE"),
            home: env::var_os("HOME"),
            appdata: env::var_os("APPDATA"),
        }
    }
}

pub(crate) fn build_config_from_process(
    database: &DatabaseConfig,
    kind: DatabaseKind,
) -> Result<Config> {
    build_config(database, kind, &CredentialSources::from_process())
}

fn build_config(
    database: &DatabaseConfig,
    kind: DatabaseKind,
    sources: &CredentialSources,
) -> Result<Config> {
    let mut config = parse_connection_config(database, kind)?;
    if config.get_user().is_none() {
        config.user(&database.username);
    }

    let explicit_password = (!database.password.is_empty()).then_some(database.password.as_bytes());
    let url_password = config.get_password().map(<[u8]>::to_vec);
    let password = resolve_password(
        explicit_password,
        url_password.as_deref(),
        sources.pgpassword.as_deref(),
        || lookup_pgpass_password(&config, sources),
    );

    if let Some(password) = password {
        config.password(password);
    }

    Ok(config)
}

fn parse_connection_config(database: &DatabaseConfig, kind: DatabaseKind) -> Result<Config> {
    if database.connection_string.starts_with("postgresql://")
        || database.connection_string.starts_with("postgres://")
    {
        return database
            .connection_string
            .parse::<Config>()
            .with_context(|| format!("Invalid {} connection string", kind.display_name()));
    }

    let target = parse_compact_target(&database.connection_string, kind)?;
    let mut config = Config::new();
    config
        .host(&target.host)
        .port(target.port)
        .dbname(&target.database)
        .user(&database.username);
    Ok(config)
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
    let host = match config.get_hosts().first()? {
        Host::Tcp(host) => host.as_bytes().to_vec(),
        #[cfg(unix)]
        Host::Unix(_) => b"localhost".to_vec(),
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
    use super::{CredentialSources, DatabaseKind, build_config, resolve_password};
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

        let config = super::build_config_from_process(&database, DatabaseKind::PostgreSql)
            .expect("explicit password should avoid lower-priority file lookup");

        assert_eq!(config.get_password(), Some(b"explicit".as_slice()));
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
