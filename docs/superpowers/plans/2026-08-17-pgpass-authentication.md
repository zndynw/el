# PostgreSQL 与 Greenplum pgpass 认证实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**目标：** 为 PostgreSQL 和 Greenplum 的 import/export 连接增加符合 libpq 规则的 password file 认证，并保持显式密码、URL 密码、`PGPASSWORD`、passfile 的确定优先级。

**架构：** 新增 `src/db/pgpass.rs` 负责字节级解析、匹配、路径选择和权限检查；新增 `src/db/postgres_config.rs` 负责把现有连接目标转成 `postgres::Config` 并解析凭据来源。PostgreSQL 与 Greenplum adapter 共用该配置构造器，应用配置解析层只保留显式密码，不再提前读取环境秘密。

**技术栈：** Rust 2024、`postgres` 0.19、`anyhow`、`tracing`、源码内联 `#[cfg(test)]` 单元测试。

---

## 文件职责

- 新建 `src/db/pgpass.rs`：passfile 记录解析、目标匹配、默认路径、Unix 权限、文件读取。
- 新建 `src/db/postgres_config.rs`：紧凑连接目标/URL 解析、凭据优先级、`postgres::Config` 构造。
- 修改 `src/db/mod.rs`：注册两个内部共享模块。
- 修改 `src/db/postgresql.rs`：使用共享 `postgres::Config` 连接。
- 修改 `src/db/greenplum.rs`：使用共享 `postgres::Config` 连接。
- 修改 `src/app/resolve.rs` 与 `src/app.rs` 测试：允许 PostgreSQL/Greenplum 执行模式缺省密码，保留 Oracle/MySQL 校验。
- 修改 `README.md`：记录路径、格式、权限与凭据优先级。

### 任务 1：实现字节级 pgpass 解析与匹配

**文件：**
- 新建：`src/db/pgpass.rs`
- 修改：`src/db/mod.rs`

- [ ] **步骤 1：写解析和匹配失败测试**

在 `src/db/mod.rs` 增加私有模块声明：

```rust
mod pgpass;
```

在 `src/db/pgpass.rs` 先定义测试所期望的接口和测试。目标结构使用借用字节，密码返回 `Vec<u8>`：

```rust
pub(crate) struct PgPassTarget<'a> {
    pub(crate) host: &'a [u8],
    pub(crate) port: u16,
    pub(crate) database: &'a [u8],
    pub(crate) user: &'a [u8],
}

fn target<'a>(host: &'a [u8]) -> PgPassTarget<'a> {
    PgPassTarget { host, port: 5432, database: b"app", user: b"alice" }
}

#[test]
fn first_matching_record_wins() {
    let input = b"db:5432:app:alice:first\ndb:5432:app:alice:second\n";
    assert_eq!(find_password(input, &target(b"db")), Some(b"first".to_vec()));
}

#[test]
fn wildcards_and_escaped_fields_match() {
    let input = b"db\\:primary:*:app:alice:pa\\:ss\\\\word\n";
    let matched = PgPassTarget { host: b"db:primary", port: 5432, database: b"app", user: b"alice" };
    assert_eq!(find_password(input, &matched), Some(b"pa:ss\\word".to_vec()));
}

#[test]
fn parser_keeps_non_utf8_password_bytes() {
    let input = b"db:5432:app:alice:\xff\xfe\n";
    assert_eq!(find_password(input, &target(b"db")), Some(vec![0xff, 0xfe]));
}
```

同时覆盖：首字符 `#` 才是注释、空行、CRLF、空密码返回 `Some(Vec::new())`、字段不足/过多被忽略、字面值不匹配。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

```text
cargo test db::pgpass::tests -- --nocapture
```

预期：编译失败，指出 `find_password` 尚未定义，证明测试命中了新行为。

- [ ] **步骤 3：实现最小解析器**

实现以下函数，不把整个文件转换成 UTF-8：

```rust
pub(crate) fn find_password(contents: &[u8], target: &PgPassTarget<'_>) -> Option<Vec<u8>>;
fn parse_record(line: &[u8]) -> Option<[Vec<u8>; 5]>;
fn field_matches(pattern: &[u8], value: &[u8]) -> bool;
```

`parse_record` 仅对 `\:` 和 `\\` 去转义；按未转义冒号切成恰好五段。`find_password` 按文件顺序扫描并立即返回第一条匹配记录，端口使用十进制 ASCII 比较。

- [ ] **步骤 4：运行 focused 与全量测试并确认 GREEN**

```text
cargo test db::pgpass::tests -- --nocapture
cargo test
```

预期：新增解析测试与原有 61 个测试全部通过。

- [ ] **步骤 5：提交解析器**

```text
git add src/db/mod.rs src/db/pgpass.rs
git commit -m "Add pgpass record parsing"
```

### 任务 2：实现 passfile 路径、权限与读取

**文件：**
- 修改：`src/db/pgpass.rs`

- [ ] **步骤 1：写路径和权限失败测试**

新增可注入平台和目录的纯函数测试：

```rust
#[derive(Clone, Copy)]
pub(crate) enum PgPassPlatform { Unix, Windows }

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
    assert_eq!(select_path(PgPassPlatform::Unix, None, Some(OsStr::new("/home/alice")), None), Some(PathBuf::from("/home/alice/.pgpass")));
    assert_eq!(select_path(PgPassPlatform::Windows, None, None, Some(OsStr::new(r"C:\Users\alice\AppData\Roaming"))), Some(PathBuf::from(r"C:\Users\alice\AppData\Roaming\postgresql\pgpass.conf")));
}

#[test]
fn unix_permission_mask_rejects_group_or_other_access() {
    assert!(unix_mode_is_secure(0o600));
    assert!(unix_mode_is_secure(0o400));
    assert!(!unix_mode_is_secure(0o640));
    assert!(!unix_mode_is_secure(0o604));
}
```

再使用唯一临时目录测试：文件缺失返回 `Ok(None)`、有效文件返回密码、无匹配返回 `Ok(None)`；Unix 下用 `PermissionsExt::set_mode` 验证 `0644` 返回不安全错误。

- [ ] **步骤 2：运行测试并确认 RED**

```text
cargo test db::pgpass::tests -- --nocapture
```

预期：缺少 `select_path`、`unix_mode_is_secure` 或 `load_password` 导致失败。

- [ ] **步骤 3：实现路径和文件读取**

实现：

```rust
pub(crate) fn select_path(
    platform: PgPassPlatform,
    pgpassfile: Option<&OsStr>,
    home: Option<&OsStr>,
    appdata: Option<&OsStr>,
) -> Option<PathBuf>;

pub(crate) fn load_password(
    path: &Path,
    target: &PgPassTarget<'_>,
) -> Result<Option<Vec<u8>>, PgPassLoadError>;

pub(crate) fn current_platform() -> PgPassPlatform;
```

`load_password` 对 `NotFound` 返回 `Ok(None)`；Unix 通过 `MetadataExt`/`PermissionsExt` 检查 `mode & 0o077 == 0`，Windows 跳过权限检查；其他读取失败和权限不安全返回只包含路径与原因、不包含文件内容的 `PgPassLoadError`。

- [ ] **步骤 4：运行测试并确认 GREEN**

```text
cargo test db::pgpass::tests -- --nocapture
cargo test
```

预期：路径、权限、读取测试及全量测试通过。

- [ ] **步骤 5：提交文件读取能力**

```text
git add src/db/pgpass.rs
git commit -m "Load passwords from pgpass files"
```

### 任务 3：实现共享 PostgreSQL 协议配置和凭据优先级

**文件：**
- 新建：`src/db/postgres_config.rs`
- 修改：`src/db/mod.rs`

- [ ] **步骤 1：写配置构造失败测试**

在 `src/db/mod.rs` 增加：

```rust
mod postgres_config;
```

在新模块中定义测试用来源对象，避免修改进程环境：

```rust
#[derive(Default)]
pub(crate) struct CredentialSources {
    pub(crate) pgpassword: Option<Vec<u8>>,
    pub(crate) pgpassfile: Option<OsString>,
    pub(crate) home: Option<OsString>,
    pub(crate) appdata: Option<OsString>,
}

#[test]
fn explicit_password_overrides_url_and_environment() {
    let database = database_config("postgresql://alice:url@db:5432/app", "explicit");
    let sources = CredentialSources { pgpassword: Some(b"environment".to_vec()), ..Default::default() };
    let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
    assert_eq!(config.get_password(), Some(b"explicit".as_slice()));
}

#[test]
fn url_password_overrides_pgpassword() {
    let database = database_config("postgresql://alice:url@db:5432/app", "");
    let sources = CredentialSources { pgpassword: Some(b"environment".to_vec()), ..Default::default() };
    let config = build_config(&database, DatabaseKind::PostgreSql, &sources).unwrap();
    assert_eq!(config.get_password(), Some(b"url".as_slice()));
}
```

继续覆盖：`PGPASSWORD` 胜过 passfile、passfile 回退、完全无密码、紧凑格式默认端口、URL host/database/user 匹配、Unix socket 按 `localhost` 匹配、Greenplum 错误名称，以及高优先级命中后不调用 passfile lookup 的纯函数测试。

- [ ] **步骤 2：运行测试并确认 RED**

```text
cargo test db::postgres_config::tests -- --nocapture
```

预期：缺少 `build_config` 与 `DatabaseKind` 导致编译失败。

- [ ] **步骤 3：实现共享配置构造器**

核心接口：

```rust
#[derive(Clone, Copy)]
pub(crate) enum DatabaseKind { PostgreSql, Greenplum }

pub(crate) fn build_config_from_process(
    database: &DatabaseConfig,
    kind: DatabaseKind,
) -> Result<postgres::Config>;

fn build_config(
    database: &DatabaseConfig,
    kind: DatabaseKind,
    sources: &CredentialSources,
) -> Result<postgres::Config>;
```

紧凑格式使用 `Config::new().host(...).port(...).dbname(...).user(...)`；URL 使用 `connection_string.parse::<postgres::Config>()`。URL 未携带 user 时使用 `DatabaseConfig.username`。按以下顺序短路：非空 `DatabaseConfig.password`、URL `get_password()`、`PGPASSWORD`、`load_password`。passfile 匹配目标取有效 host（Unix socket 为 `localhost`）、首个 port 或 5432、dbname 或 user、user。passfile 读取错误通过 `tracing::warn!` 只记录路径和错误类别。

进程来源使用 `var_os("PGPASSFILE")`、`var_os("HOME")`、`var_os("APPDATA")` 和 `var("PGPASSWORD")`；这些调用只发生在 adapter 的 `connect` 阶段。

- [ ] **步骤 4：运行测试并确认 GREEN**

```text
cargo test db::postgres_config::tests -- --nocapture
cargo test
```

预期：所有优先级、目标解析和原有测试通过。

- [ ] **步骤 5：提交共享配置构造器**

```text
git add src/db/mod.rs src/db/postgres_config.rs
git commit -m "Resolve PostgreSQL connection credentials"
```

### 任务 4：接入 PostgreSQL、Greenplum 与 CLI 配置流

**文件：**
- 修改：`src/db/postgresql.rs`
- 修改：`src/db/greenplum.rs`
- 修改：`src/app/resolve.rs`
- 修改：`src/app.rs`

- [ ] **步骤 1：写 CLI 执行模式失败测试**

在 `src/app.rs` 增加 PostgreSQL 和 Greenplum 非 dry-run 测试：

```rust
#[test]
fn export_execution_allows_postgresql_without_password() {
    let mut args = empty_args();
    args.db_type = Some("postgresql".to_string());
    args.conn = Some("localhost:5432/app".to_string());
    args.username = Some("app".to_string());
    let config = build_database_config_from_args(&args).expect("pgpass may supply password later");
    assert!(config.password.is_empty());
}

#[test]
fn export_execution_allows_greenplum_without_password() {
    let mut args = empty_args();
    args.db_type = Some("greenplum".to_string());
    args.conn = Some("localhost:5432/app".to_string());
    args.username = Some("app".to_string());
    let config = build_database_config_from_args(&args).expect("pgpass may supply password later");
    assert!(config.password.is_empty());
}
```

增加对应 import 测试，并保留/增加 Oracle、MySQL 无密码仍报错的断言。

- [ ] **步骤 2：运行测试并确认 RED**

```text
cargo test app::tests::export_execution_allows_postgresql_without_password -- --nocapture
```

预期：当前提前返回 `Password is required`，测试失败。

- [ ] **步骤 3：移除应用层环境读取和提前错误**

在 `merge_database_config_import`、`merge_database_config` 中仅在 `args.password` 为 `Some` 时覆盖密码。`build_database_config_from_args_import` 与 `build_database_config_from_args` 对 PostgreSQL/Greenplum 无显式密码统一返回空字符串；Oracle/MySQL 继续走现有必填错误。删除 PostgreSQL/Greenplum 的 `Password is required. Use --password ...` 提前校验。

- [ ] **步骤 4：让两个 adapter 使用共享配置**

PostgreSQL：

```rust
fn connect(&mut self) -> Result<()> {
    let config = build_config_from_process(&self.config, DatabaseKind::PostgreSql)?;
    let client = config.connect(NoTls)
        .context("Failed to connect to PostgreSQL database")?;
    self.connection = Some(client);
    Ok(())
}
```

Greenplum 使用相同代码但传 `DatabaseKind::Greenplum` 并保留现有错误上下文。删除两个文件中重复的 `build_connection_string`、target struct 和 `parse_connection_target`。

- [ ] **步骤 5：运行 focused 与全量测试并确认 GREEN**

```text
cargo test app::tests -- --nocapture
cargo test db::postgres_config::tests -- --nocapture
cargo test
```

预期：import/export 对 PostgreSQL/Greenplum 都允许延迟认证，Oracle/MySQL 行为不变，所有测试通过。

- [ ] **步骤 6：提交集成改动**

```text
git add src/app.rs src/app/resolve.rs src/db/postgresql.rs src/db/greenplum.rs
git commit -m "Use pgpass for PostgreSQL connections"
```

### 任务 5：更新文档并完成验证

**文件：**
- 修改：`README.md`

- [ ] **步骤 1：更新用户文档**

将 PostgreSQL/Greenplum 密码说明改为：显式 `password`/`--password` > URL 密码 > `PGPASSWORD` > password file。记录 Unix `~/.pgpass`、Windows `%APPDATA%\postgresql\pgpass.conf`、`PGPASSFILE`、五字段格式、首条匹配、Unix 权限要求，并强调日志不显示密码。

- [ ] **步骤 2：格式化并检查差异**

```text
cargo fmt
cargo fmt --check
git diff --check
```

预期：三条命令退出码均为 0。

- [ ] **步骤 3：运行完整验证**

```text
cargo test
cargo build
```

预期：所有测试 0 failed，debug build 成功且没有新增 warning。

- [ ] **步骤 4：检查凭据泄漏与范围**

```text
rg -n "PGPASSWORD|PGPASSFILE|pgpass|password" src README.md
git diff --stat master...HEAD
git status --short
```

确认所有日志仅包含路径/来源/错误类别，不包含密码；变更仅覆盖 spec、plan、pgpass/连接配置、adapter、CLI 解析和 README。

- [ ] **步骤 5：提交文档与最终整理**

```text
git add README.md
git commit -m "Document pgpass authentication"
```
