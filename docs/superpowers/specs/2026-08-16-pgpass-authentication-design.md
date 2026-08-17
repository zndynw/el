# PostgreSQL and Greenplum pgpass Authentication Design

**Issue:** [#17](https://github.com/zndynw/el/issues/17)

**Goal:** Allow PostgreSQL and Greenplum import and export connections to authenticate through a libpq-compatible password file when no password is supplied by a higher-priority source.

## Scope

The feature applies to PostgreSQL and Greenplum connections created by both import and export flows. It supports:

- Unix default path: `$HOME/.pgpass`
- Windows default path: `%APPDATA%\postgresql\pgpass.conf`
- `PGPASSFILE` as an override for the default path
- libpq password-file field matching, wildcards, escaping, first-match behavior, and Unix permission checks
- Existing compact connection targets (`host:port/database` and `host/database`) and PostgreSQL URLs accepted by the application

The feature does not add a new CLI option, connection service files, replication connections, or general libpq connection-string compatibility beyond formats already accepted by the application. `PGPASSFILE` is the supported explicit passfile-path override.

## Credential Precedence

Credentials are selected in this order:

1. `--password` or TOML `database.password`
2. A password embedded in a PostgreSQL URL
3. `PGPASSWORD`
4. The first matching password-file entry

The first available source wins. Lower-priority sources are not read after a password has been selected. This prevents unnecessary access to secret files and preserves explicit user intent.

## Architecture

Add `src/db/pgpass.rs` as a focused internal module. It owns password-file path selection, permission validation, parsing, matching, and password lookup. Its parsing and matching APIs operate on supplied strings, paths, and connection attributes so tests do not mutate process-wide environment variables.

Add a shared PostgreSQL protocol connection-config helper under `src/db/`. PostgreSQL and Greenplum adapters use it to convert the application's existing connection target into `postgres::Config`, determine the effective host, port, database, and user, apply credential precedence, and set the selected password through `postgres::Config::password`. Each adapter retains its database-specific connection error context and all query/import behavior.

The application resolution layer stops copying `PGPASSWORD` into `DatabaseConfig.password`. That field represents only an explicit CLI or TOML password. This source distinction is necessary to preserve precedence against URL passwords and to avoid showing an environment-derived password in resolved configuration output.

## Password-File Semantics

Each non-comment record contains five colon-separated fields:

```text
hostname:port:database:username:password
```

Blank lines and lines whose first character is `#` are ignored. A backslash escapes a colon or backslash within a field. The first four fields may contain `*`, which matches any value. Matching is performed against host, port, database, and user, in that order. The first matching record wins, including a record with an empty password.

For TCP connections, the effective hostname is matched as configured. For an empty host or a Unix-domain socket host, the password-file hostname is matched as `localhost`, following libpq behavior. The default port is `5432` when none is specified.

The file is parsed as bytes so a non-UTF-8 password does not invalidate the file; connection attributes are compared using their encoded bytes and the selected password is passed to `postgres::Config::password` without logging or lossy conversion. Malformed records are ignored without exposing their contents. CRLF and LF line endings are accepted.

On Unix, the password file is used only when group and other permission bits are all clear. A file with permissions such as `0644` is ignored and a warning names only the path and reason. On Windows, no permission-bit check is performed. A missing default file or a file with no matching entry is normal and produces no warning. An explicitly selected but unreadable file may produce a warning that excludes file contents and credentials.

## Data Flow

1. Import or export resolves CLI and TOML values into `DatabaseConfig`; missing passwords are valid for PostgreSQL and Greenplum in executing and non-executing modes.
2. Dry-run and resolved-config printing stop before database adapter connection and therefore do not inspect `PGPASSWORD` or a password file.
3. At connection time, the shared helper parses the connection target into `postgres::Config` and extracts the effective matching attributes.
4. The helper checks explicit configuration, URL password, `PGPASSWORD`, then the selected password file.
5. If a password is found, the helper applies it through `postgres::Config::password` and returns the config.
6. If no password is found, the helper returns the config without a password. The driver then produces its normal authentication/configuration error if the server requires one.
7. PostgreSQL or Greenplum calls `Config::connect(NoTls)` and adds its existing database-specific error context.

No password is added to a loggable connection string. Existing debug and resolved-config redaction remains in place, and new diagnostics never include a password or full password-file record.

## Error Handling

- Missing default password file: continue without a password.
- No matching record: continue without a password.
- Unsafe Unix permissions: ignore the file and emit a redacted warning.
- Unreadable explicitly selected file: ignore it and emit a redacted warning.
- Malformed record: ignore that record and continue scanning.
- Invalid existing connection target: return the current PostgreSQL- or Greenplum-specific validation error.
- Server requires a password but no source supplies one: return the driver connection error rather than the application's early `Password is required` error.

## Testing

### Parser and Matching

Unit tests in `src/db/pgpass.rs` cover blank lines, comments only at the first character, exactly five fields, escaped colons, escaped backslashes, wildcards, literal matching, first-match behavior, empty and non-UTF-8 passwords, malformed records, CRLF, and Unix-socket-to-`localhost` matching.

### Paths and Permissions

Tests pass environment values and base directories into path-selection helpers. They cover `PGPASSFILE` precedence, Unix `$HOME/.pgpass`, and Windows `%APPDATA%\postgresql\pgpass.conf`. Unix-only tests create files with safe and unsafe modes; Windows tests confirm permission bits are not consulted.

### Credential Resolution

Shared connection-config tests cover the full precedence chain, compact and URL connection targets, default port handling, no-match behavior, and the rule that lower-priority sources are not accessed once a password is selected.

### Application Regression

Inline tests in the existing application resolution module cover PostgreSQL and Greenplum import/export without `--password`, confirm that the early export error is removed, and verify that Oracle and MySQL password requirements are unchanged. Existing redaction tests remain and are extended where needed to ensure environment and password-file values are never rendered.

Every production behavior change follows red-green-refactor: add one focused failing test, verify the expected failure, implement the minimum behavior, then rerun the focused and complete test suites.

## Verification

Before completion, run:

```text
cargo fmt --check
cargo test
cargo build
```

The feature is complete when all tests pass on the development platform, platform-specific code compiles under its target configuration, PostgreSQL and Greenplum share the same credential behavior, and README documentation describes supported paths and precedence.
