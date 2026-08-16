pub(crate) struct PgPassTarget<'a> {
    pub(crate) host: &'a [u8],
    pub(crate) port: u16,
    pub(crate) database: &'a [u8],
    pub(crate) user: &'a [u8],
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
    use super::{PgPassTarget, find_password};

    fn target(host: &[u8]) -> PgPassTarget<'_> {
        PgPassTarget {
            host,
            port: 5432,
            database: b"app",
            user: b"alice",
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
}
