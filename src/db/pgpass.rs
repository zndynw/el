use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct PgPassTarget<'a> {
    pub(crate) host: &'a [u8],
    pub(crate) port: u16,
    pub(crate) database: &'a [u8],
    pub(crate) user: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PgPassPlatform {
    #[cfg(any(not(windows), test))]
    Unix,
    #[cfg(any(windows, test))]
    Windows,
}

#[derive(Debug)]
pub(crate) enum PgPassLoadError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    #[cfg(unix)]
    UnsafePermissions {
        path: PathBuf,
    },
}

impl fmt::Display for PgPassLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            #[cfg(unix)]
            Self::UnsafePermissions { path } => write!(
                formatter,
                "ignored {} because group or other permissions are set",
                path.display()
            ),
        }
    }
}

impl Error for PgPassLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            #[cfg(unix)]
            Self::UnsafePermissions { .. } => None,
        }
    }
}

pub(crate) fn select_path(
    platform: PgPassPlatform,
    pgpassfile: Option<&OsStr>,
    home: Option<&OsStr>,
    appdata: Option<&OsStr>,
) -> Option<PathBuf> {
    #[cfg(all(windows, not(test)))]
    let _ = home;
    #[cfg(all(not(windows), not(test)))]
    let _ = appdata;

    if let Some(path) = pgpassfile {
        return Some(PathBuf::from(path));
    }

    match platform {
        #[cfg(any(not(windows), test))]
        PgPassPlatform::Unix => home.map(|path| PathBuf::from(path).join(".pgpass")),
        #[cfg(any(windows, test))]
        PgPassPlatform::Windows => {
            appdata.map(|path| PathBuf::from(path).join("postgresql").join("pgpass.conf"))
        }
    }
}

pub(crate) fn load_password(
    path: &Path,
    target: &PgPassTarget<'_>,
) -> Result<Option<Vec<u8>>, PgPassLoadError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(PgPassLoadError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if !unix_mode_is_secure(metadata.permissions().mode()) {
            return Err(PgPassLoadError::UnsafePermissions {
                path: path.to_path_buf(),
            });
        }
    }

    #[cfg(not(unix))]
    let _ = metadata;

    let contents = fs::read(path).map_err(|source| PgPassLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(find_password(&contents, target))
}

#[cfg(any(unix, test))]
pub(crate) fn unix_mode_is_secure(mode: u32) -> bool {
    mode & 0o077 == 0
}

pub(crate) fn find_password(contents: &[u8], target: &PgPassTarget<'_>) -> Option<Vec<u8>> {
    let port = target.port.to_string();

    for line in contents.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.first() == Some(&b'#') {
            continue;
        }

        let fields = match parse_record(line) {
            Some(fields) => fields,
            None => continue,
        };

        if field_matches(&fields[0], target.host)
            && field_matches(&fields[1], port.as_bytes())
            && field_matches(&fields[2], target.database)
            && field_matches(&fields[3], target.user)
        {
            return Some(fields[4].clone());
        }
    }

    None
}

fn parse_record(line: &[u8]) -> Option<[Vec<u8>; 5]> {
    let mut fields = vec![Vec::new()];
    let mut index = 0;

    while index < line.len() {
        let byte = line[index];
        if byte == b'\\' && index + 1 < line.len() && matches!(line[index + 1], b':' | b'\\') {
            fields.last_mut()?.push(line[index + 1]);
            index += 2;
            continue;
        }

        if byte == b':' {
            if fields.len() == 5 {
                return None;
            }
            fields.push(Vec::new());
        } else {
            fields.last_mut()?.push(byte);
        }
        index += 1;
    }

    fields.try_into().ok()
}

fn field_matches(pattern: &[u8], value: &[u8]) -> bool {
    pattern == b"*" || pattern == value
}

#[cfg(test)]
mod tests {
    use super::{
        PgPassPlatform, PgPassTarget, find_password, load_password, select_path,
        unix_mode_is_secure,
    };
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn target(host: &[u8]) -> PgPassTarget<'_> {
        PgPassTarget {
            host,
            port: 5432,
            database: b"app",
            user: b"alice",
        }
    }

    fn temp_path(test_name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "el-pgpass-{}-{test_name}-{nonce}",
            std::process::id()
        ))
    }

    fn write_test_pgpass(path: &PathBuf, contents: &[u8]) {
        fs::write(path, contents).expect("test pgpass should be written");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("test pgpass permissions should be set");
        }
    }

    #[test]
    fn first_matching_record_wins() {
        let input = b"db:5432:app:alice:first\ndb:5432:app:alice:second\n";

        assert_eq!(
            find_password(input, &target(b"db")),
            Some(b"first".to_vec())
        );
    }

    #[test]
    fn wildcards_and_escaped_fields_match() {
        let input = b"db\\:primary:*:app:alice:pa\\:ss\\\\word\n";
        let matched = PgPassTarget {
            host: b"db:primary",
            port: 5432,
            database: b"app",
            user: b"alice",
        };

        assert_eq!(
            find_password(input, &matched),
            Some(b"pa:ss\\word".to_vec())
        );
    }

    #[test]
    fn parser_keeps_non_utf8_password_bytes() {
        let input = b"db:5432:app:alice:\xff\xfe\n";

        assert_eq!(find_password(input, &target(b"db")), Some(vec![0xff, 0xfe]));
    }

    #[test]
    fn comments_blank_lines_and_crlf_are_ignored() {
        let input = b"# ignored\r\n\r\ndb:5432:app:alice:secret\r\n";

        assert_eq!(
            find_password(input, &target(b"db")),
            Some(b"secret".to_vec())
        );
    }

    #[test]
    fn hash_after_first_character_is_not_a_comment() {
        let input = b" #db:5432:app:alice:secret\n";

        assert_eq!(
            find_password(input, &target(b" #db")),
            Some(b"secret".to_vec())
        );
    }

    #[test]
    fn empty_password_is_a_match() {
        let input = b"db:5432:app:alice:\n";

        assert_eq!(find_password(input, &target(b"db")), Some(Vec::new()));
    }

    #[test]
    fn malformed_records_are_skipped() {
        let input = b"db:5432:app:missing\ndb:5432:app:alice:too:many\ndb:5432:app:alice:valid\n";

        assert_eq!(
            find_password(input, &target(b"db")),
            Some(b"valid".to_vec())
        );
    }

    #[test]
    fn literal_fields_must_all_match() {
        let input = b"db:5433:app:alice:wrong-port\ndb:5432:other:alice:wrong-db\ndb:5432:app:bob:wrong-user\n";

        assert_eq!(find_password(input, &target(b"db")), None);
    }

    #[test]
    fn pgpassfile_overrides_platform_default() {
        let path = select_path(
            PgPassPlatform::Unix,
            Some(OsStr::new("custom.pass")),
            Some(OsStr::new("/home/alice")),
            None,
        );

        assert_eq!(path, Some(PathBuf::from("custom.pass")));
    }

    #[test]
    fn platform_defaults_match_libpq() {
        assert_eq!(
            select_path(
                PgPassPlatform::Unix,
                None,
                Some(OsStr::new("/home/alice")),
                None,
            ),
            Some(PathBuf::from("/home/alice/.pgpass"))
        );
        assert_eq!(
            select_path(
                PgPassPlatform::Windows,
                None,
                None,
                Some(OsStr::new(r"C:\Users\alice\AppData\Roaming")),
            ),
            Some(PathBuf::from(
                r"C:\Users\alice\AppData\Roaming\postgresql\pgpass.conf"
            ))
        );
    }

    #[test]
    fn unix_permission_mask_rejects_group_or_other_access() {
        assert!(unix_mode_is_secure(0o600));
        assert!(unix_mode_is_secure(0o400));
        assert!(!unix_mode_is_secure(0o640));
        assert!(!unix_mode_is_secure(0o604));
    }

    #[test]
    fn missing_file_returns_no_password() {
        let path = temp_path("missing");

        assert_eq!(load_password(&path, &target(b"db")).unwrap(), None);
    }

    #[test]
    fn file_lookup_returns_matching_password() {
        let path = temp_path("matching");
        write_test_pgpass(&path, b"db:5432:app:alice:file-secret\n");

        let password = load_password(&path, &target(b"db")).unwrap();
        fs::remove_file(&path).expect("test pgpass should be removed");

        assert_eq!(password, Some(b"file-secret".to_vec()));
    }

    #[test]
    fn file_lookup_returns_none_without_a_match() {
        let path = temp_path("no-match");
        write_test_pgpass(&path, b"other:5432:app:alice:file-secret\n");

        let password = load_password(&path, &target(b"db")).unwrap();
        fs::remove_file(&path).expect("test pgpass should be removed");

        assert_eq!(password, None);
    }

    #[cfg(unix)]
    #[test]
    fn file_lookup_rejects_unsafe_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_path("unsafe-mode");
        fs::write(&path, b"db:5432:app:alice:file-secret\n")
            .expect("test pgpass should be written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("test permissions should be set");

        let error = load_password(&path, &target(b"db")).expect_err("mode should be rejected");
        fs::remove_file(&path).expect("test pgpass should be removed");

        assert!(error.to_string().contains("permissions"));
        assert!(!error.to_string().contains("file-secret"));
    }
}
