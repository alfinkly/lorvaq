# lorvaq — план 2: Postgres, администратор и настройки

> **Для агентов-исполнителей:** ОБЯЗАТЕЛЬНЫЙ ПОД-СКИЛЛ: используйте superpowers:subagent-driven-development (рекомендуется) или superpowers:executing-plans, чтобы выполнять план задача за задачей. Шаги размечены чекбоксами (`- [ ]`).

**Цель:** `lorvaq server` без `--demo` работает с Postgres: пишет историю событий, хранит настройки, при первом запуске печатает одноразовый код, по которому в интерфейсе создаётся пароль администратора; администратор входит, выбирает, кто видит схемы (`open` / `proxy` / `admin`), меняет настройки сцены CI на лету и срок хранения истории. `docker compose up` поднимает lorvaq вместе с Postgres.

**Архитектура:** новые модули `crypto` (шифрование секретов, пароли, токены, код настройки), `store` (Postgres через `sqlx`, миграции вшиты), `history` (пакетная запись событий сцены в базу, не тормозит живую схему; чистка по сроку), `auth` (режимы доступа, ограничение попыток входа, код настройки). Сцена получает `settings()` / `configure()`, runtime принимает новые настройки без перезапуска. HTTP делится на публичные маршруты, маршруты зрителя (за охранником режима доступа) и маршруты администратора (за охранником сессии). Фронт: страницы «Настройка», «Вход», «Настройки».

**Стек:** добавляются sqlx 0.8 (postgres, migrate, json), argon2 0.5, chacha20poly1305 0.10 (XChaCha20-Poly1305), base64 0.22, sha2 0.10, ipnet 2, axum-extra 0.10 (cookie), time 0.3; для тестов testcontainers-modules 0.13 (postgres). Все версии проверены компиляцией на rustc 1.91, тест с настоящим Postgres в testcontainers проходит.

**Spec:** `docs/design/2026-10-10-core-and-ci-scene.md` — §5 (страницы настройки и настроек), §6.1 (администратор, сессии, режимы зрителей), §6.3 (env и шифрование секретов), §7 (`/api/v1/admin/...`), §8 (хранение, история 14 дней, живая схема без базы), §9 (база недоступна), §10 (тесты на Postgres в testcontainers).

## Глобальные ограничения

- Всё из глобальных ограничений плана 1 остаётся в силе: Rust 1.91, clippy `-D warnings` и fmt чистые, ни следа конкретной студии, UI на английском со строками в `web/src/i18n.ts`, время — миллисекунды Unix, коммиты на английском с последней строкой `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Обязательный env для живого режима — ровно два: `DATABASE_URL`, `LORVAQ_SECRET_KEY` (32 байта в base64). Необязательные: `LORVAQ_PUBLIC_URL`, `LORVAQ_LISTEN`. Всё остальное настраивается в интерфейсе.
- `--demo` работает как раньше: без базы, без администратора, режим `open`; маршруты администратора отвечают 404.
- Запросы к базе — только `sqlx::query*` во время выполнения, без макросов `query!` (сборка не должна требовать базу). Миграции — `sqlx::migrate!("./migrations")`, вшиты в бинарник.
- Пароли — argon2id, длина ≥ **12** символов, хэширование и проверка — в `spawn_blocking`. Сессия — случайный токен 32 байта, в базе только SHA-256, cookie `lorvaq_session` (`HttpOnly`, `SameSite=Lax`, `Path=/`, 30 дней, `Secure` при `LORVAQ_PUBLIC_URL` на `https://`). Код настройки — 16 символов `XXXX-XXXX-XXXX-XXXX`, в базе только хэш, живёт 24 часа.
- Вход: не больше **5** неудачных попыток с одного адреса за **15** минут, дальше 429.
- Режим доступа по умолчанию — `admin` (безопасный по умолчанию).
- История: очередь записи не больше **10 000** событий на сцену, пачки по **500**, запись раз в секунду; при переполнении отбрасываются старые; хранение по умолчанию **14** дней (допустимо 1–365), чистка раз в час.
- Секреты не печатаются в лог (исключение — одноразовый код настройки, это его назначение); ошибки базы — в лог, клиенту — `503 {"error":"the database is unavailable"}`.
- Ответы ошибок API — `{"error": "<текст>"}`.

## Фокус ревью

1. **База недоступна при работающем сервере** — живая схема продолжает работать, запись истории копит очередь и пишет ошибку в статус, API настроек отвечает 503, а не падает (задачи 4 и 6).
2. **Сброс пароля, пока сервер работает** (`docker exec … lorvaq admin reset-password`) — все сессии сразу недействительны, новый код настройки работает без перезапуска сервера (задачи 5 и 7).
3. **Режим `proxy` и запрос не от доверенного прокси** — заголовок пользователя от постороннего адреса не пускает (задача 5, тест по матрице).
4. **Неверные настройки сцены** (битое регулярное выражение в выходе) — 400 с текстом, старые настройки остаются и в сцене, и в базе (задачи 3 и 6).
5. **Перебор пароля** — шестая попытка за 15 минут получает 429 даже с верным паролем (задача 6).

## Структура файлов

```text
Cargo.toml                       + зависимости
migrations/0001_init.sql         таблицы: settings, admin, sessions, scene_settings, events
src/crypto.rs                    SecretBox, пароли, токены, код настройки
src/cli.rs                       server / admin reset-password / keygen (заменяется целиком)
src/store/mod.rs                 Store: настройки, админ, сессии, настройки сцен, события
src/scene.rs                     + settings()/configure(), Control, runtime без входов (заменяется целиком)
src/scenes/ci/reduce.rs          + settings()/configure() у CiScene
src/history.rs                   Writer, spawn_writer, spawn_pruner, HistorySettings
src/auth.rs                      Access, LoginLimiter, код настройки
src/http/mod.rs                  AppState/Live, маршруты (заменяется целиком)
src/http/guard.rs                Peer, охранники зрителя и администратора
src/http/account.rs              me, setup, login, logout, change_password
src/http/admin.rs                доступ, история, настройки сцен
src/app.rs                       live_state, run_server, reset_password (заменяется целиком)
src/main.rs                      (заменяется целиком)
tests/common/mod.rs              общий Postgres в testcontainers, вызовы роутера
tests/store.rs  tests/http_live.rs  tests/live_smoke.rs
web/src/api.ts                   + вход, настройки, AuthError, состояние «locked» (заменяется целиком)
web/src/settings/forms.ts (+ .test.ts)   проверки форм
web/src/pages/Setup.tsx Login.tsx Settings.tsx
web/src/App.tsx  web/src/i18n.ts  web/src/index.css  web/src/scenes/ci/CiView.tsx
Dockerfile  compose.yml  README.md
```

---

### Задача 1: Криптография и командная строка

**Files:**
- Modify: `Cargo.toml` — добавить зависимости (все сразу, чтобы следующие задачи только писали код)
- Create: `src/crypto.rs`
- Replace: `src/cli.rs`, `src/main.rs` (временный — задача 7 заменит его целиком)
- Modify: `src/lib.rs` — добавить `pub mod crypto;`

**Interfaces:**
- Produces:
  - `crypto::SecretBox::{from_base64(&str) -> anyhow::Result<Self>, generate_key() -> String, seal(&[u8]) -> String, open(&str) -> anyhow::Result<Vec<u8>>}`; `SecretBox: Clone`
  - `crypto::{hash_password(&str) -> String, verify_password(&str, &str) -> bool, random_token() -> String, token_hash(&str) -> String, setup_code() -> String}`
  - `cli::{Cli, Command::{Server(ServerArgs), Admin { action: AdminAction, db: DbArgs }, Keygen}, ServerArgs { demo, listen, public_url: Option<String>, db: DbArgs, secret_key: Option<String> }, DbArgs { database_url: Option<String> }, AdminAction::ResetPassword}`

- [ ] **Step 1: Добавить зависимости**

В `Cargo.toml`, секция `[dependencies]` — добавить (в алфавитном порядке с существующими):

```toml
argon2 = "0.5"
axum-extra = { version = "0.10", features = ["cookie"] }
base64 = "0.22"
chacha20poly1305 = "0.10"
ipnet = "2"
sha2 = "0.10"
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "postgres", "migrate", "macros", "json"] }
time = "0.3"
```

Секция `[dev-dependencies]` — добавить:

```toml
testcontainers-modules = { version = "0.13", features = ["postgres"] }
```

Run: `cargo build`
Expected: сборка проходит (новые крейты пока не используются — предупреждений об этом Cargo не даёт).

- [ ] **Step 2: Написать падающие тесты криптографии**

`src/crypto.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_values_open_with_the_same_key_only() {
        let key = SecretBox::generate_key();
        let a = SecretBox::from_base64(&key).unwrap();
        let sealed = a.seal(b"token");
        assert_ne!(sealed, a.seal(b"token"), "a fresh nonce every time");
        assert_eq!(a.open(&sealed).unwrap(), b"token");
        let b = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
        assert!(b.open(&sealed).is_err());
    }

    #[test]
    fn damaged_values_and_bad_keys_are_errors() {
        let a = SecretBox::from_base64(&SecretBox::generate_key()).unwrap();
        let mut sealed = a.seal(b"token").into_bytes();
        let last = sealed.len() - 2;
        sealed[last] = if sealed[last] == b'A' { b'B' } else { b'A' };
        assert!(a.open(&String::from_utf8(sealed).unwrap()).is_err());
        assert!(a.open("c2hvcnQ=").is_err(), "shorter than a nonce");
        assert!(a.open("not base64!").is_err());
        assert!(SecretBox::from_base64("c2hvcnQ=").is_err(), "not 32 bytes");
        assert!(SecretBox::from_base64("%%%").is_err());
    }

    #[test]
    fn passwords_verify_only_themselves() {
        let hash = hash_password("correct horse battery");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("correct horse batterY", &hash));
        assert!(!verify_password("anything", "not a hash"));
    }

    #[test]
    fn tokens_are_random_and_hash_stably() {
        let (a, b) = (random_token(), random_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(token_hash("x"), token_hash("x"));
        assert_eq!(token_hash("x").len(), 64);
        assert_ne!(token_hash("x"), token_hash("y"));
    }

    #[test]
    fn setup_codes_are_typable() {
        let code = setup_code();
        assert_eq!(code.len(), 19);
        let groups: Vec<&str> = code.split('-').collect();
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|g| g.len() == 4 && g.chars().all(|c| SETUP_ALPHABET.contains(&(c as u8)))));
        assert_ne!(code, setup_code());
    }
}
```

- [ ] **Step 3: Запустить — падает**

Run: `cargo test crypto::` (после добавления `pub mod crypto;` в `src/lib.rs`)
Expected: ошибки компиляции `cannot find struct 'SecretBox'`.

- [ ] **Step 4: Реализовать**

Добавить в начало `src/crypto.rs`:

```rust
//! Secrets at rest (LORVAQ_SECRET_KEY), admin passwords, session tokens, setup codes.

use anyhow::{anyhow, bail, Context};
use argon2::password_hash::rand_core::OsRng as PasswordRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};

const NONCE_LEN: usize = 24;
/// No 0/O, 1/I/L: easy to read off a terminal and type.
pub const SETUP_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// Encrypts secrets kept in the database (GitLab tokens and the like).
#[derive(Clone)]
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl SecretBox {
    pub fn from_base64(key: &str) -> anyhow::Result<Self> {
        let bytes = STANDARD.decode(key.trim()).context("LORVAQ_SECRET_KEY is not valid base64")?;
        if bytes.len() != 32 {
            bail!("LORVAQ_SECRET_KEY must be 32 bytes, got {}", bytes.len());
        }
        Ok(Self { cipher: XChaCha20Poly1305::new(Key::from_slice(&bytes)) })
    }

    pub fn generate_key() -> String {
        STANDARD.encode(XChaCha20Poly1305::generate_key(&mut OsRng))
    }

    /// base64(nonce || ciphertext).
    pub fn seal(&self, plain: &[u8]) -> String {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let mut out = nonce.to_vec();
        out.extend(self.cipher.encrypt(&nonce, plain).expect("encryption with a valid key cannot fail"));
        STANDARD.encode(out)
    }

    pub fn open(&self, sealed: &str) -> anyhow::Result<Vec<u8>> {
        let bytes = STANDARD.decode(sealed).context("the sealed value is not base64")?;
        if bytes.len() < NONCE_LEN {
            bail!("the sealed value is too short");
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        self.cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| anyhow!("cannot decrypt: another LORVAQ_SECRET_KEY or a damaged value"))
    }
}

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut PasswordRng);
    Argon2::default().hash_password(password.as_bytes(), &salt).expect("argon2 with default parameters cannot fail").to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

/// 32 random bytes, base64url without padding (43 characters).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// What the database keeps instead of a token: hex SHA-256.
pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// `XXXX-XXXX-XXXX-XXXX`.
pub fn setup_code() -> String {
    let mut rng = rand::rng();
    let chars: Vec<char> = (0..16).map(|_| SETUP_ALPHABET[rng.random_range(0..SETUP_ALPHABET.len())] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}
```

- [ ] **Step 5: Написать падающие тесты CLI и заменить `src/cli.rs` целиком**

`src/cli.rs`:

```rust
//! Command line: `lorvaq server`, `lorvaq admin reset-password`, `lorvaq keygen`.

use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;

#[derive(Debug, Parser)]
#[command(name = "lorvaq", version, about = "Live scenes for real-time monitoring")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server: sources, scenes, API and UI.
    Server(ServerArgs),
    /// Administration that works on the database directly.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
        #[command(flatten)]
        db: DbArgs,
    },
    /// Print a new random LORVAQ_SECRET_KEY.
    Keygen,
}

#[derive(Debug, Args)]
pub struct ServerArgs {
    /// Use the built-in simulator instead of real sources (no database needed).
    #[arg(long)]
    pub demo: bool,
    /// Address to listen on.
    #[arg(long, env = "LORVAQ_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: SocketAddr,
    /// The URL people open lorvaq at; https:// turns on the Secure cookie flag.
    #[arg(long, env = "LORVAQ_PUBLIC_URL")]
    pub public_url: Option<String>,
    #[command(flatten)]
    pub db: DbArgs,
    /// 32 bytes in base64 that encrypt stored secrets (`lorvaq keygen`).
    #[arg(long, env = "LORVAQ_SECRET_KEY", hide_env_values = true)]
    pub secret_key: Option<String>,
}

#[derive(Debug, Args)]
pub struct DbArgs {
    /// Postgres connection URL.
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    pub database_url: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum AdminAction {
    /// Remove the admin password, sign everybody out and print a new one-time setup code.
    ResetPassword,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_demo_with_listen() {
        let cli = Cli::try_parse_from(["lorvaq", "server", "--demo", "--listen", "127.0.0.1:9000"]).unwrap();
        let Command::Server(args) = cli.command else { panic!("server expected") };
        assert!(args.demo);
        assert_eq!(args.listen.port(), 9000);
    }

    #[test]
    fn server_takes_database_and_key_from_flags() {
        let cli = Cli::try_parse_from(["lorvaq", "server", "--database-url", "postgres://x", "--secret-key", "k", "--public-url", "https://l"]).unwrap();
        let Command::Server(args) = cli.command else { panic!("server expected") };
        assert!(!args.demo);
        assert_eq!(args.db.database_url.as_deref(), Some("postgres://x"));
        assert_eq!(args.secret_key.as_deref(), Some("k"));
        assert_eq!(args.public_url.as_deref(), Some("https://l"));
    }

    #[test]
    fn admin_reset_password_and_keygen() {
        let cli = Cli::try_parse_from(["lorvaq", "admin", "reset-password", "--database-url", "postgres://x"]).unwrap();
        let Command::Admin { action: AdminAction::ResetPassword, db } = cli.command else { panic!("admin expected") };
        assert_eq!(db.database_url.as_deref(), Some("postgres://x"));
        assert!(matches!(Cli::try_parse_from(["lorvaq", "keygen"]).unwrap().command, Command::Keygen));
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(Cli::try_parse_from(["lorvaq", "nope"]).is_err());
    }
}
```

Заменить `src/main.rs` целиком (временный — живой режим и сброс пароля появятся в задаче 7):

```rust
use clap::Parser;
use lorvaq::app;
use lorvaq::cli::{Cli, Command};
use lorvaq::crypto::SecretBox;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();
    match Cli::parse().command {
        Command::Keygen => {
            println!("{}", SecretBox::generate_key());
            Ok(())
        }
        Command::Server(args) => {
            anyhow::ensure!(args.demo, "the live mode is wired in a later step of this plan; use --demo");
            app::serve(args.listen, app::demo_scenes()?).await
        }
        Command::Admin { .. } => anyhow::bail!("admin commands are wired in a later step of this plan"),
    }
}
```

- [ ] **Step 6: Запустить — тесты проходят**

Run: `cargo test crypto:: cli:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check && cargo run -q -- keygen`
Expected: `9 passed`; `keygen` печатает строку base64 длиной 44.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/
git commit -m "feat: secret box, passwords, tokens and the admin/keygen commands

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 2: Хранилище в Postgres

**Files:**
- Create: `migrations/0001_init.sql`, `src/store/mod.rs`, `tests/common/mod.rs`, `tests/store.rs`
- Modify: `src/lib.rs` — добавить `pub mod store;`

**Interfaces:**
- Consumes: `world::Event`.
- Produces: `#[derive(Clone)] store::Store` с методами:
  `connect(&str)`, `ping() -> bool`, `close()`,
  `setting::<T>(key) -> Result<Option<T>>`, `put_setting(key, &T)`, `delete_setting(key)`,
  `admin_hash() -> Result<Option<String>>`, `set_admin_hash(&str, now_ms)`, `reset_admin()`,
  `create_session(token_hash, now_ms, ttl_ms)`, `session_valid(token_hash, now_ms) -> Result<bool>`, `delete_session(token_hash)`, `delete_other_sessions(keep_hash)`, `prune_sessions(now_ms) -> Result<u64>`,
  `scene_settings(scene) -> Result<Option<Value>>`, `put_scene_settings(scene, &Value, now_ms)`,
  `insert_events(scene, &[Event])`, `prune_events(older_than_ms) -> Result<u64>`, `event_count(scene) -> Result<i64>`.
  Все методы возвращают `anyhow::Result`.
- Produces (тесты): `tests/common::{store() -> Store, store_with_url() -> (Store, String), call(...)}`.

- [ ] **Step 1: Миграция**

`migrations/0001_init.sql`:

```sql
-- lorvaq schema v1: settings, the single admin, sessions, scene settings, event history.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value JSONB NOT NULL
);

CREATE TABLE admin (
    id            SMALLINT PRIMARY KEY CHECK (id = 1),
    password_hash TEXT NOT NULL,
    updated_ms    BIGINT NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    created_ms BIGINT NOT NULL,
    expires_ms BIGINT NOT NULL
);

CREATE TABLE scene_settings (
    scene      TEXT PRIMARY KEY,
    value      JSONB NOT NULL,
    updated_ms BIGINT NOT NULL
);

-- seq restarts with the server, so it is not part of the key.
CREATE TABLE events (
    id     BIGSERIAL PRIMARY KEY,
    scene  TEXT NOT NULL,
    seq    BIGINT NOT NULL,
    ts_ms  BIGINT NOT NULL,
    change JSONB NOT NULL
);
CREATE INDEX events_scene_ts ON events (scene, ts_ms);
```

- [ ] **Step 2: Тестовая обвязка и падающие тесты**

`tests/common/mod.rs`:

```rust
//! Shared helpers for tests that need Postgres (docker must be running).
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use lorvaq::store::Store;
use serde_json::Value;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;
use tokio::sync::OnceCell;
use tower::ServiceExt;

static PG: OnceCell<(ContainerAsync<Postgres>, String)> = OnceCell::const_new();

/// A fresh, migrated database; one Postgres container per test binary.
pub async fn store_with_url() -> (Store, String) {
    let (_, base) = PG
        .get_or_init(|| async {
            let node = Postgres::default().start().await.expect("docker is needed for database tests");
            let host = node.get_host().await.expect("container host");
            let port = node.get_host_port_ipv4(5432).await.expect("container port");
            (node, format!("postgres://postgres:postgres@{host}:{port}"))
        })
        .await;
    let name: String = format!("t{}", lorvaq::crypto::token_hash(&lorvaq::crypto::random_token())).chars().take(16).collect();
    let admin = sqlx::PgPool::connect(&format!("{base}/postgres")).await.expect("postgres is up");
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.expect("create a test database");
    admin.close().await;
    let url = format!("{base}/{name}");
    (Store::connect(&url).await.expect("migrations run"), url)
}

pub async fn store() -> Store {
    store_with_url().await.0
}

/// One request through the router: status, JSON body (or null), the session cookie it set (`name=value`).
pub async fn call(router: &Router, method: &str, uri: &str, cookie: Option<&str>, body: Option<Value>, headers: &[(&str, &str)]) -> (StatusCode, Value, Option<String>) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = match body {
        Some(b) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())).unwrap(),
        None => req.body(Body::empty()).unwrap(),
    };
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), cookie)
}
```

`tests/store.rs`:

```rust
mod common;

use lorvaq::store::Store;
use lorvaq::world::{Change, Event};
use serde_json::json;

#[tokio::test]
async fn migrations_can_run_again() {
    let (_, url) = common::store_with_url().await;
    Store::connect(&url).await.expect("second start on the same database");
}

#[tokio::test]
async fn settings_round_trip() {
    let s = common::store().await;
    assert!(s.ping().await);
    assert_eq!(s.setting::<i64>("x").await.unwrap(), None);
    s.put_setting("x", &5i64).await.unwrap();
    s.put_setting("x", &7i64).await.unwrap();
    assert_eq!(s.setting::<i64>("x").await.unwrap(), Some(7));
    assert!(s.setting::<String>("x").await.is_err(), "wrong shape is an error, not a silent default");
    s.delete_setting("x").await.unwrap();
    assert_eq!(s.setting::<i64>("x").await.unwrap(), None);
}

#[tokio::test]
async fn admin_and_sessions() {
    let s = common::store().await;
    assert_eq!(s.admin_hash().await.unwrap(), None);
    s.set_admin_hash("h1", 1).await.unwrap();
    s.set_admin_hash("h2", 2).await.unwrap();
    assert_eq!(s.admin_hash().await.unwrap().as_deref(), Some("h2"));

    s.create_session("a", 100, 50).await.unwrap();
    s.create_session("b", 100, 50).await.unwrap();
    assert!(s.session_valid("a", 149).await.unwrap());
    assert!(!s.session_valid("a", 150).await.unwrap(), "expired");
    assert!(!s.session_valid("nope", 120).await.unwrap());

    s.delete_other_sessions("a").await.unwrap();
    assert!(s.session_valid("a", 120).await.unwrap());
    assert!(!s.session_valid("b", 120).await.unwrap());
    s.delete_session("a").await.unwrap();
    assert!(!s.session_valid("a", 120).await.unwrap());

    s.create_session("c", 100, 50).await.unwrap();
    s.reset_admin().await.unwrap();
    assert_eq!(s.admin_hash().await.unwrap(), None);
    assert!(!s.session_valid("c", 120).await.unwrap(), "reset signs everybody out");
}

#[tokio::test]
async fn expired_sessions_are_pruned() {
    let s = common::store().await;
    s.create_session("old", 0, 10).await.unwrap();
    s.create_session("new", 0, 1_000).await.unwrap();
    assert_eq!(s.prune_sessions(500).await.unwrap(), 1);
    assert!(s.session_valid("new", 500).await.unwrap());
}

#[tokio::test]
async fn scene_settings_round_trip() {
    let s = common::store().await;
    assert_eq!(s.scene_settings("ci").await.unwrap(), None);
    s.put_scene_settings("ci", &json!({"max_projects": 3}), 1).await.unwrap();
    s.put_scene_settings("ci", &json!({"max_projects": 4}), 2).await.unwrap();
    assert_eq!(s.scene_settings("ci").await.unwrap(), Some(json!({"max_projects": 4})));
}

#[tokio::test]
async fn events_are_written_counted_and_pruned() {
    let s = common::store().await;
    s.insert_events("ci", &[]).await.unwrap();
    let ev = |seq: u64, ts_ms: i64| Event { seq, ts_ms, change: Change::Gone { id: format!("job:{seq}") } };
    s.insert_events("ci", &[ev(1, 100), ev(2, 200), ev(3, 300)]).await.unwrap();
    s.insert_events("other", &[ev(1, 100)]).await.unwrap();
    assert_eq!(s.event_count("ci").await.unwrap(), 3);
    assert_eq!(s.prune_events(250).await.unwrap(), 3, "two of ci and one of other are older");
    assert_eq!(s.event_count("ci").await.unwrap(), 1);
}
```

Run: `cargo test --test store`
Expected: ошибка компиляции `could not find 'store' in 'lorvaq'`.

- [ ] **Step 3: Реализовать хранилище**

`src/store/mod.rs`:

```rust
//! Postgres: settings, the admin, sessions, scene settings and event history.

use crate::world::Event;
use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, QueryBuilder};
use std::time::Duration;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connects and brings the schema up to date.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await
            .context("cannot connect to the database (DATABASE_URL)")?;
        MIGRATOR.run(&pool).await.context("database migration failed")?;
        Ok(Self { pool })
    }

    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    /// Closes the pool: every later query fails (shutdown, and tests of a down database).
    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn setting<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        let value: Option<Value> = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1").bind(key).fetch_optional(&self.pool).await?;
        value.map(|v| serde_json::from_value(v).with_context(|| format!("setting {key:?} has an unexpected shape"))).transpose()
    }

    pub async fn put_setting<T: Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value")
            .bind(key)
            .bind(serde_json::to_value(value)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_setting(&self, key: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM settings WHERE key = $1").bind(key).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn admin_hash(&self) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT password_hash FROM admin WHERE id = 1").fetch_optional(&self.pool).await?)
    }

    pub async fn set_admin_hash(&self, hash: &str, now_ms: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO admin (id, password_hash, updated_ms) VALUES (1, $1, $2) \
             ON CONFLICT (id) DO UPDATE SET password_hash = EXCLUDED.password_hash, updated_ms = EXCLUDED.updated_ms",
        )
        .bind(hash)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Removes the admin and signs everybody out.
    pub async fn reset_admin(&self) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM admin").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM sessions").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_session(&self, token_hash: &str, now_ms: i64, ttl_ms: i64) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES ($1, $2, $3)")
            .bind(token_hash)
            .bind(now_ms)
            .bind(now_ms + ttl_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn session_valid(&self, token_hash: &str, now_ms: i64) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sessions WHERE token_hash = $1 AND expires_ms > $2)")
            .bind(token_hash)
            .bind(now_ms)
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn delete_session(&self, token_hash: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1").bind(token_hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_other_sessions(&self, keep_hash: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash <> $1").bind(keep_hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn prune_sessions(&self, now_ms: i64) -> anyhow::Result<u64> {
        Ok(sqlx::query("DELETE FROM sessions WHERE expires_ms <= $1").bind(now_ms).execute(&self.pool).await?.rows_affected())
    }

    pub async fn scene_settings(&self, scene: &str) -> anyhow::Result<Option<Value>> {
        Ok(sqlx::query_scalar("SELECT value FROM scene_settings WHERE scene = $1").bind(scene).fetch_optional(&self.pool).await?)
    }

    pub async fn put_scene_settings(&self, scene: &str, value: &Value, now_ms: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO scene_settings (scene, value, updated_ms) VALUES ($1, $2, $3) \
             ON CONFLICT (scene) DO UPDATE SET value = EXCLUDED.value, updated_ms = EXCLUDED.updated_ms",
        )
        .bind(scene)
        .bind(value)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_events(&self, scene: &str, events: &[Event]) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut query = QueryBuilder::<Postgres>::new("INSERT INTO events (scene, seq, ts_ms, change) ");
        query.push_values(events, |mut row, ev| {
            row.push_bind(scene)
                .push_bind(ev.seq as i64)
                .push_bind(ev.ts_ms)
                .push_bind(serde_json::to_value(&ev.change).expect("changes always serialize"));
        });
        query.build().execute(&self.pool).await?;
        Ok(())
    }

    pub async fn prune_events(&self, older_than_ms: i64) -> anyhow::Result<u64> {
        Ok(sqlx::query("DELETE FROM events WHERE ts_ms < $1").bind(older_than_ms).execute(&self.pool).await?.rows_affected())
    }

    pub async fn event_count(&self, scene: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM events WHERE scene = $1").bind(scene).fetch_one(&self.pool).await?)
    }
}
```

Добавить в `src/lib.rs` строку `pub mod store;`.

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cargo test --test store && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `6 passed` (первый запуск скачивает образ `postgres` — до минуты).

- [ ] **Step 5: Commit**

```bash
git add migrations/ src/ tests/
git commit -m "feat(store): Postgres schema and store for settings, admin, sessions, history

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 3: Настройки сцены на лету

**Files:**
- Replace: `src/scene.rs`
- Modify: `src/scenes/ci/reduce.rs` — два метода в `impl Scene for CiScene` и тест

**Interfaces:**
- Consumes: `CiSettings`, `CiSettings::compile_exits`.
- Produces:
  - трейт `Scene` получает `fn settings(&self) -> serde_json::Value` и `fn configure(&mut self, settings: Value) -> anyhow::Result<()>`;
  - `scene::LOCK: &str` — общий текст для `expect` на блокировках;
  - `SceneHandle::{settings() -> Value, configure(Value).await -> anyhow::Result<Value>}` (возвращает применённые, нормализованные настройки); `SceneHandle::new` — handle без runtime, `configure` у него — ошибка;
  - runtime продолжает тикать и принимать настройки, когда все отправители входов закрыты.

- [ ] **Step 1: Заменить `src/scene.rs` целиком (с тестами)**

```rust
//! A scene turns source inputs into world changes. The runtime owns the scene's
//! world, applies the changes, publishes the valid ones and takes new settings.

use crate::world::bus::{EventBus, BUFFER};
use crate::world::{Change, Snapshot, World};
use anyhow::{anyhow, Context};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};

/// Our locks are never held across code that can panic.
pub const LOCK: &str = "lock is never poisoned: no panics while held";

pub trait Scene: Send + 'static {
    type Input: Send + 'static;
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// Pure: reads the world, returns the changes. Must not block.
    fn reduce(&mut self, world: &World, input: Self::Input, now_ms: i64) -> Vec<Change>;
    /// Time-based changes: timeouts, cleanup. Called about once a second.
    fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change>;
    /// Current settings, as the settings page shows them.
    fn settings(&self) -> Value;
    /// Replaces the settings and keeps the world. On error the old settings stay.
    fn configure(&mut self, settings: Value) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct SceneState {
    pub world: World,
    pub seq: u64,
}

/// Asks the runtime to apply new settings; the reply carries what was applied.
pub struct Control {
    pub settings: Value,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

#[derive(Clone)]
pub struct SceneHandle {
    pub id: &'static str,
    pub title: &'static str,
    pub state: Arc<RwLock<SceneState>>,
    pub bus: Arc<EventBus>,
    settings: Arc<RwLock<Value>>,
    control: Option<mpsc::Sender<Control>>,
}

impl SceneHandle {
    /// A handle without a runtime: it can be committed to but not configured.
    pub fn new(id: &'static str, title: &'static str, cap: usize) -> Self {
        Self {
            id,
            title,
            state: Arc::new(RwLock::new(SceneState::default())),
            bus: Arc::new(EventBus::new(cap)),
            settings: Arc::new(RwLock::new(Value::Null)),
            control: None,
        }
    }

    /// World and seq read under one lock, so the snapshot matches its seq exactly.
    pub fn snapshot(&self) -> Snapshot {
        let st = self.state.read().expect(LOCK);
        st.world.snapshot(st.seq)
    }

    /// Applies changes in order; the valid ones become events. Returns how many.
    pub fn commit(&self, changes: Vec<Change>, now_ms: i64) -> usize {
        let mut st = self.state.write().expect(LOCK);
        let mut applied = 0;
        for change in changes {
            if st.world.apply(&change, now_ms) {
                st.seq = self.bus.publish(now_ms, change).seq;
                applied += 1;
            }
        }
        applied
    }

    pub fn settings(&self) -> Value {
        self.settings.read().expect(LOCK).clone()
    }

    /// Applies new settings in the running scene; returns them as applied.
    pub async fn configure(&self, settings: Value) -> anyhow::Result<Value> {
        let control = self.control.as_ref().context("this scene has no runtime to configure")?;
        let (reply, answer) = oneshot::channel();
        control.send(Control { settings, reply }).await.map_err(|_| anyhow!("the scene has stopped"))?;
        answer.await.map_err(|_| anyhow!("the scene has stopped"))?.map_err(|e| anyhow!(e))
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub fn spawn<S: Scene>(mut scene: S, tick_every: Duration) -> (SceneHandle, mpsc::Sender<S::Input>) {
    let (control_tx, mut control_rx) = mpsc::channel::<Control>(8);
    let mut handle = SceneHandle::new(scene.id(), scene.title(), BUFFER);
    handle.control = Some(control_tx);
    *handle.settings.write().expect(LOCK) = scene.settings();
    let (tx, mut rx) = mpsc::channel::<S::Input>(256);
    let h = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tick_every);
        let mut inputs_open = true;
        loop {
            tokio::select! {
                input = rx.recv(), if inputs_open => match input {
                    Some(input) => {
                        let now = now_ms();
                        let changes = {
                            let st = h.state.read().expect(LOCK);
                            scene.reduce(&st.world, input, now)
                        };
                        h.commit(changes, now);
                    }
                    // No source feeds this scene (yet): keep ticking and taking settings.
                    None => inputs_open = false,
                },
                control = control_rx.recv() => {
                    // The handle keeps a sender, so this only ends with the process.
                    let Some(Control { settings, reply }) = control else { break };
                    let result = scene.configure(settings).map(|()| scene.settings()).map_err(|e| format!("{e:#}"));
                    if let Ok(applied) = &result {
                        *h.settings.write().expect(LOCK) = applied.clone();
                    }
                    let _ = reply.send(result);
                }
                _ = tick.tick() => {
                    let now = now_ms();
                    let changes = {
                        let st = h.state.read().expect(LOCK);
                        scene.tick(&st.world, now)
                    };
                    h.commit(changes, now);
                }
            }
        }
    });
    (handle, tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Attrs, Entity, Place};
    use serde_json::json;

    fn zone(id: &str) -> Change {
        Change::UpsertPlace {
            place: Place { id: id.into(), kind: "zone".into(), parent: None, label: id.into(), capacity: None, attrs: Attrs::new() },
        }
    }

    /// Input: a place id to create. Tick: creates "ticked" once. Settings: {"name": String}.
    struct Toy {
        name: String,
    }

    impl Scene for Toy {
        type Input = String;
        fn id(&self) -> &'static str {
            "toy"
        }
        fn title(&self) -> &'static str {
            "Toy"
        }
        fn reduce(&mut self, _world: &World, input: String, _now_ms: i64) -> Vec<Change> {
            vec![zone(&input)]
        }
        fn tick(&mut self, world: &World, _now_ms: i64) -> Vec<Change> {
            if world.place("ticked").is_none() { vec![zone("ticked")] } else { vec![] }
        }
        fn settings(&self) -> Value {
            json!({ "name": self.name })
        }
        fn configure(&mut self, settings: Value) -> anyhow::Result<()> {
            self.name = settings.get("name").and_then(Value::as_str).context("name must be a string")?.to_string();
            Ok(())
        }
    }

    fn toy() -> Toy {
        Toy { name: "a".into() }
    }

    #[test]
    fn commit_publishes_only_valid_changes() {
        let h = SceneHandle::new("t", "T", 100);
        let moved = Change::Move { id: "nobody".into(), to: "a".into() };
        assert_eq!(h.commit(vec![zone("a"), zone("a"), moved], 1), 1);
        assert_eq!(h.bus.last_seq(), 1);
        assert_eq!(h.snapshot().seq, 1);
        assert_eq!(h.snapshot().places.len(), 1);
    }

    #[test]
    fn snapshot_plus_events_rebuild_the_world() {
        let h = SceneHandle::new("t", "T", 100);
        h.commit(vec![zone("a"), zone("b")], 1);
        let snap = h.snapshot();
        let job = Entity { id: "j".into(), kind: "job".into(), place: "a".into(), state: "pending".into(), attrs: Attrs::new(), since_ms: 2 };
        h.commit(vec![zone("c"), Change::Appear { entity: job }], 2);
        h.commit(vec![Change::Move { id: "j".into(), to: "c".into() }], 3);
        let mut rebuilt = World::from_snapshot(&snap);
        for ev in h.bus.since(snap.seq).unwrap() {
            assert!(rebuilt.apply(&ev.change, ev.ts_ms), "event {} replays", ev.seq);
        }
        assert_eq!(rebuilt.snapshot(h.bus.last_seq()), h.snapshot());
    }

    #[tokio::test]
    async fn runtime_feeds_inputs_and_ticks() {
        let (h, tx) = spawn(toy(), Duration::from_millis(20));
        assert_eq!((h.id, h.title), ("toy", "Toy"));
        tx.send("queue".into()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        let snap = h.snapshot();
        let ids: Vec<&str> = snap.places.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"queue") && ids.contains(&"ticked"), "{ids:?}");
    }

    #[tokio::test]
    async fn runtime_applies_settings_and_keeps_them_on_error() {
        let (h, _tx) = spawn(toy(), Duration::from_millis(20));
        assert_eq!(h.settings(), json!({ "name": "a" }));
        assert_eq!(h.configure(json!({ "name": "b" })).await.unwrap(), json!({ "name": "b" }));
        assert_eq!(h.settings(), json!({ "name": "b" }));
        let err = h.configure(json!(42)).await.unwrap_err();
        assert!(err.to_string().contains("name must be a string"), "{err}");
        assert_eq!(h.settings(), json!({ "name": "b" }), "old settings stay");
    }

    #[tokio::test]
    async fn a_scene_without_sources_keeps_ticking_and_configuring() {
        let (h, tx) = spawn(toy(), Duration::from_millis(20));
        drop(tx);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(h.snapshot().places.iter().any(|p| p.id == "ticked"));
        assert!(h.configure(json!({ "name": "c" })).await.is_ok());
    }

    #[tokio::test]
    async fn a_handle_without_a_runtime_cannot_be_configured() {
        let h = SceneHandle::new("t", "T", 10);
        assert!(h.configure(json!({})).await.is_err());
        assert_eq!(h.settings(), Value::Null);
    }
}
```

- [ ] **Step 2: Написать падающий тест сцены CI**

Дописать в модуль `tests` файла `src/scenes/ci/reduce.rs`:

```rust
    #[test]
    fn configure_keeps_jobs_adds_exits_and_rejects_bad_patterns() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        assert_eq!(h.scene.settings()["max_projects"], serde_json::json!(8));
        h.scene.configure(serde_json::json!({ "exits": [{ "label": "prod", "stage": "deploy" }] })).unwrap();
        h.tick(0);
        assert!(h.world.place("exit:prod").is_some(), "the new exit appears on the next tick");
        assert!(h.place_of(1).is_some(), "the world is kept");
        let bad = serde_json::json!({ "exits": [{ "label": "x", "stage": "(" }] });
        assert!(h.scene.configure(bad).is_err());
        assert_eq!(h.scene.settings()["exits"][0]["label"], serde_json::json!("prod"), "old settings stay");
    }
```

Run: `cargo test scenes::ci && cargo test scene::`
Expected: ошибка компиляции `not all trait items implemented, missing: 'settings', 'configure'` для `CiScene`.

- [ ] **Step 3: Реализовать у сцены CI**

В `src/scenes/ci/reduce.rs`, внутри `impl Scene for CiScene`, после `fn title(...)`, добавить:

```rust
    fn settings(&self) -> serde_json::Value {
        serde_json::to_value(&self.settings).expect("settings always serialize")
    }

    fn configure(&mut self, settings: serde_json::Value) -> anyhow::Result<()> {
        let settings: CiSettings = serde_json::from_value(settings)?;
        let exits = settings.compile_exits()?;
        self.settings = settings;
        self.exits = exits;
        Ok(())
    }
```

- [ ] **Step 4: Запустить — всё проходит**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: все тесты зелёные, включая 6 в `scene::` и новый в `scenes::ci`.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(scene): settings and live reconfiguration, runtime without sources

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 4: История событий в базе

**Files:**
- Create: `src/history.rs`
- Modify: `src/lib.rs` — добавить `pub mod history;`

**Interfaces:**
- Consumes: `Store::{insert_events, prune_events, prune_sessions, setting}`, `SceneHandle::bus`, `scene::{now_ms, LOCK}`.
- Produces:
  - `history::{QUEUE_CAP = 10_000, BATCH = 500, HISTORY_KEY = "history"}`
  - `#[derive(Clone, Copy, Serialize, Deserialize)] HistorySettings { retention_days: i64 }` (по умолчанию 14) + `validate() -> Result<(), String>` (1..=365)
  - `#[derive(Clone, Default, Serialize)] HistoryStatus { queued: usize, dropped: u64, written: u64, last_error: Option<String> }`
  - `trait EventSink` (реализован для `Store`), `Writer<S: EventSink>::{new, push, lost, flush}`
  - `spawn_writer(Store, SceneHandle, Arc<Mutex<HistoryStatus>>)`, `spawn_pruner(Store)`

- [ ] **Step 1: Написать падающие тесты**

`src/history.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Change;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct FakeSink {
        down: AtomicBool,
        seen: Mutex<Vec<Vec<u64>>>,
    }

    impl EventSink for Arc<FakeSink> {
        async fn write(&self, scene: &str, events: &[Event]) -> anyhow::Result<()> {
            assert_eq!(scene, "ci");
            if self.down.load(Ordering::SeqCst) {
                anyhow::bail!("connection refused");
            }
            self.seen.lock().unwrap().push(events.iter().map(|e| e.seq).collect());
            Ok(())
        }
    }

    fn ev(seq: u64) -> Event {
        Event { seq, ts_ms: 0, change: Change::Gone { id: "j".into() } }
    }

    fn writer() -> (Writer<Arc<FakeSink>>, Arc<FakeSink>, Arc<Mutex<HistoryStatus>>) {
        let sink = Arc::new(FakeSink::default());
        let status = Arc::new(Mutex::new(HistoryStatus::default()));
        (Writer::new(sink.clone(), "ci", status.clone()), sink, status)
    }

    #[tokio::test]
    async fn writes_in_batches_and_empties_the_queue() {
        let (mut w, sink, status) = writer();
        for seq in 1..=1_200 {
            w.push(ev(seq));
        }
        assert!(w.flush().await);
        let sizes: Vec<usize> = sink.seen.lock().unwrap().iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![BATCH, BATCH, 200]);
        let st = status.lock().unwrap().clone();
        assert_eq!((st.queued, st.written, st.last_error), (0, 1_200, None));
    }

    #[tokio::test]
    async fn a_failing_database_keeps_events_in_order_until_it_is_back() {
        let (mut w, sink, status) = writer();
        sink.down.store(true, Ordering::SeqCst);
        w.push(ev(1));
        w.push(ev(2));
        assert!(!w.flush().await);
        assert!(status.lock().unwrap().last_error.as_deref().unwrap().contains("connection refused"));
        assert_eq!(status.lock().unwrap().queued, 2);
        sink.down.store(false, Ordering::SeqCst);
        w.push(ev(3));
        assert!(w.flush().await);
        assert_eq!(sink.seen.lock().unwrap().concat(), vec![1, 2, 3]);
        assert_eq!(status.lock().unwrap().last_error, None);
    }

    #[tokio::test]
    async fn overflow_drops_the_oldest_and_counts_them() {
        let (mut w, sink, status) = writer();
        for seq in 1..=(QUEUE_CAP as u64 + 3) {
            w.push(ev(seq));
        }
        w.lost(5);
        assert_eq!(status.lock().unwrap().dropped, 8);
        assert!(w.flush().await);
        let all = sink.seen.lock().unwrap().concat();
        assert_eq!((all.len(), all[0]), (QUEUE_CAP, 4));
    }

    #[test]
    fn retention_defaults_and_bounds() {
        let s: HistorySettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.retention_days, 14);
        assert!(s.validate().is_ok());
        assert!(HistorySettings { retention_days: 0 }.validate().is_err());
        assert!(HistorySettings { retention_days: 366 }.validate().is_err());
    }
}
```

Run: `cargo test history::` (после `pub mod history;` в `src/lib.rs`)
Expected: ошибки компиляции `cannot find trait 'EventSink'`.

- [ ] **Step 2: Реализовать**

Добавить в начало `src/history.rs`:

```rust
//! Each scene's events go to Postgres in batches. The live scene never waits for
//! the database: a down database only grows a bounded queue.

use crate::scene::{now_ms, SceneHandle, LOCK};
use crate::store::Store;
use crate::world::Event;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

pub const QUEUE_CAP: usize = 10_000;
pub const BATCH: usize = 500;
pub const HISTORY_KEY: &str = "history";
const DAY_MS: i64 = 24 * 3600 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistorySettings {
    pub retention_days: i64,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { retention_days: 14 }
    }
}

impl HistorySettings {
    pub fn validate(&self) -> Result<(), String> {
        if (1..=365).contains(&self.retention_days) { Ok(()) } else { Err("keep history from 1 to 365 days".into()) }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HistoryStatus {
    pub queued: usize,
    pub dropped: u64,
    pub written: u64,
    pub last_error: Option<String>,
}

pub trait EventSink: Send + Sync + 'static {
    fn write(&self, scene: &str, events: &[Event]) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl EventSink for Store {
    async fn write(&self, scene: &str, events: &[Event]) -> anyhow::Result<()> {
        self.insert_events(scene, events).await
    }
}

pub struct Writer<S> {
    sink: S,
    scene: &'static str,
    queue: VecDeque<Event>,
    status: Arc<Mutex<HistoryStatus>>,
}

impl<S: EventSink> Writer<S> {
    pub fn new(sink: S, scene: &'static str, status: Arc<Mutex<HistoryStatus>>) -> Self {
        Self { sink, scene, queue: VecDeque::new(), status }
    }

    pub fn push(&mut self, ev: Event) {
        let mut st = self.status.lock().expect(LOCK);
        if self.queue.len() == QUEUE_CAP {
            self.queue.pop_front();
            st.dropped += 1;
        }
        self.queue.push_back(ev);
        st.queued = self.queue.len();
    }

    /// Events the bus dropped before we could read them.
    pub fn lost(&mut self, n: u64) {
        self.status.lock().expect(LOCK).dropped += n;
    }

    /// Writes the queue in batches. False when the sink failed; the rest stays queued.
    pub async fn flush(&mut self) -> bool {
        while !self.queue.is_empty() {
            let n = self.queue.len().min(BATCH);
            let batch: Vec<Event> = self.queue.iter().take(n).cloned().collect();
            let result = self.sink.write(self.scene, &batch).await;
            let mut st = self.status.lock().expect(LOCK);
            match result {
                Ok(()) => {
                    self.queue.drain(..n);
                    st.written += n as u64;
                    st.queued = self.queue.len();
                }
                Err(e) => {
                    st.last_error = Some(format!("{e:#}"));
                    return false;
                }
            }
        }
        self.status.lock().expect(LOCK).last_error = None;
        true
    }
}

/// Follows a scene's bus and writes its events, once a second.
pub fn spawn_writer(store: Store, scene: SceneHandle, status: Arc<Mutex<HistoryStatus>>) {
    tokio::spawn(async move {
        // Subscribe first, then take what the bus already holds: the scene may
        // have published before this task started.
        let mut rx = scene.bus.subscribe();
        let mut writer = Writer::new(store, scene.id, status);
        let mut written_up_to = 0;
        for ev in scene.bus.since(0).unwrap_or_default() {
            written_up_to = ev.seq;
            writer.push(ev);
        }
        let mut every = tokio::time::interval(Duration::from_secs(1));
        let mut healthy = true;
        loop {
            tokio::select! {
                received = rx.recv() => match received {
                    Ok(ev) if ev.seq > written_up_to => writer.push(ev),
                    Ok(_) => {}
                    Err(RecvError::Lagged(n)) => writer.lost(n),
                    Err(RecvError::Closed) => break,
                },
                _ = every.tick() => {
                    let ok = writer.flush().await;
                    if ok != healthy {
                        healthy = ok;
                        if ok {
                            tracing::info!(scene = scene.id, "history is written again");
                        } else {
                            tracing::warn!(scene = scene.id, "cannot write history; events wait in memory");
                        }
                    }
                }
            }
        }
    });
}

/// Hourly: drop history older than the retention and expired sessions.
pub fn spawn_pruner(store: Store) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(3600));
        loop {
            every.tick().await;
            let settings = store.setting::<HistorySettings>(HISTORY_KEY).await.ok().flatten().unwrap_or_default();
            let now = now_ms();
            match store.prune_events(now - settings.retention_days * DAY_MS).await {
                Ok(n) if n > 0 => tracing::info!(removed = n, "old history removed"),
                Ok(_) => {}
                Err(e) => tracing::warn!("cannot prune history: {e:#}"),
            }
            if let Err(e) = store.prune_sessions(now).await {
                tracing::warn!("cannot prune sessions: {e:#}");
            }
        }
    });
}
```

- [ ] **Step 3: Запустить — тесты проходят**

Run: `cargo test history:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `4 passed`.

- [ ] **Step 4: Commit**

```bash
git add src/
git commit -m "feat(history): batched event history that never blocks the live scene

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 5: Доступ — режимы, попытки входа, код настройки

**Files:**
- Create: `src/auth.rs`
- Modify: `src/lib.rs` — добавить `pub mod auth;`; `tests/store.rs` — тест кода настройки

**Interfaces:**
- Consumes: `crypto::{setup_code, token_hash}`, `Store::{setting, put_setting, admin_hash}`.
- Produces:
  - `auth::{ACCESS_KEY = "access", SETUP_CODE_KEY = "setup_code"}`
  - `#[derive(Default)] enum Access { Open, Proxy { header: String, trusted: Vec<String> }, #[default] Admin }` — serde `tag = "mode"`, snake_case → JSON `{"mode":"open"}`, `{"mode":"proxy","header":"x-user","trusted":["10.0.0.0/8"]}`, `{"mode":"admin"}`; методы `validate() -> Result<(), String>`, `viewer_allowed(Option<IpAddr>, &HeaderMap, admin: bool) -> bool`
  - `#[derive(Default)] LoginLimiter::{blocked(ip, now_ms) -> bool, fail(ip, now_ms), reset(ip)}`
  - `issue_setup_code(&Store, now_ms) -> Result<String>`, `check_setup_code(&Store, code, now_ms) -> Result<bool>`, `announce_setup_code(&Store, now_ms) -> Result<()>`

- [ ] **Step 1: Написать падающие тесты**

`src/auth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn proxy(trusted: &[&str]) -> Access {
        Access::Proxy { header: "x-forwarded-user".into(), trusted: trusted.iter().map(|s| s.to_string()).collect() }
    }

    fn with_user() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-user", HeaderValue::from_static("dana"));
        h
    }

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn json_shape_is_the_api_contract() {
        assert_eq!(serde_json::to_value(Access::Admin).unwrap(), serde_json::json!({ "mode": "admin" }));
        assert_eq!(Access::default(), Access::Admin);
        let p: Access = serde_json::from_value(serde_json::json!({ "mode": "proxy", "header": "x-u", "trusted": ["10.0.0.1"] })).unwrap();
        assert_eq!(p, Access::Proxy { header: "x-u".into(), trusted: vec!["10.0.0.1".into()] });
    }

    #[test]
    fn validation() {
        assert!(Access::Open.validate().is_ok());
        assert!(proxy(&["10.0.0.0/8", "192.168.1.5", "::1"]).validate().is_ok());
        assert!(proxy(&[]).validate().is_err(), "no trusted proxy");
        assert!(proxy(&["10.0.0.0/33"]).validate().is_err());
        assert!(proxy(&["proxy.local"]).validate().is_err(), "names are not addresses");
        let bad_header = Access::Proxy { header: "bad header".into(), trusted: vec!["10.0.0.1".into()] };
        assert!(bad_header.validate().is_err());
        let empty_header = Access::Proxy { header: String::new(), trusted: vec!["10.0.0.1".into()] };
        assert!(empty_header.validate().is_err());
    }

    #[test]
    fn viewer_matrix() {
        let none = HeaderMap::new();
        assert!(Access::Open.viewer_allowed(None, &none, false));
        assert!(!Access::Admin.viewer_allowed(ip("10.0.0.1"), &with_user(), false));
        assert!(Access::Admin.viewer_allowed(None, &none, true), "the admin always sees");
        let p = proxy(&["10.0.0.0/8"]);
        assert!(p.viewer_allowed(ip("10.1.2.3"), &with_user(), false));
        assert!(!p.viewer_allowed(ip("10.1.2.3"), &none, false), "no user header");
        assert!(!p.viewer_allowed(ip("203.0.113.9"), &with_user(), false), "header from a stranger");
        assert!(!p.viewer_allowed(None, &with_user(), false), "unknown peer");
        let mut empty = HeaderMap::new();
        empty.insert("x-forwarded-user", HeaderValue::from_static(""));
        assert!(!p.viewer_allowed(ip("10.1.2.3"), &empty, false));
    }

    #[test]
    fn five_failures_in_fifteen_minutes_block_the_address() {
        let mut l = LoginLimiter::default();
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        for t in 0..5 {
            assert!(!l.blocked(a, t));
            l.fail(a, t);
        }
        assert!(l.blocked(a, 10));
        assert!(!l.blocked(b, 10), "other addresses are fine");
        assert!(!l.blocked(a, 15 * 60 * 1000 + 5), "the window slides");
        l.fail(a, 0);
        l.reset(a);
        assert!(!l.blocked(a, 1));
    }

    #[test]
    fn setup_codes_ignore_case_and_dashes() {
        assert_eq!(normalize(" abcd-efgh-2345 6789 "), "ABCDEFGH23456789");
    }
}
```

Дописать в `tests/store.rs`:

```rust
#[tokio::test]
async fn setup_codes_are_one_time_and_expire() {
    let s = common::store().await;
    assert!(!lorvaq::auth::check_setup_code(&s, "AAAA-AAAA-AAAA-AAAA", 0).await.unwrap(), "no code issued");
    let code = lorvaq::auth::issue_setup_code(&s, 1_000).await.unwrap();
    assert!(lorvaq::auth::check_setup_code(&s, &code, 2_000).await.unwrap());
    assert!(lorvaq::auth::check_setup_code(&s, &code.to_lowercase().replace('-', " "), 2_000).await.unwrap());
    assert!(!lorvaq::auth::check_setup_code(&s, "WRONG-CODE", 2_000).await.unwrap());
    assert!(!lorvaq::auth::check_setup_code(&s, &code, 1_000 + 24 * 3600 * 1000).await.unwrap(), "expired");
    let newer = lorvaq::auth::issue_setup_code(&s, 3_000).await.unwrap();
    assert!(!lorvaq::auth::check_setup_code(&s, &code, 4_000).await.unwrap(), "only the newest code works");
    assert!(lorvaq::auth::check_setup_code(&s, &newer, 4_000).await.unwrap());
}

#[tokio::test]
async fn a_setup_code_is_announced_only_without_an_admin() {
    let s = common::store().await;
    lorvaq::auth::announce_setup_code(&s, 0).await.unwrap();
    assert!(s.setting::<serde_json::Value>(lorvaq::auth::SETUP_CODE_KEY).await.unwrap().is_some());
    let t = common::store().await;
    t.set_admin_hash("h", 0).await.unwrap();
    lorvaq::auth::announce_setup_code(&t, 0).await.unwrap();
    assert!(t.setting::<serde_json::Value>(lorvaq::auth::SETUP_CODE_KEY).await.unwrap().is_none());
}
```

Run: `cargo test auth::` (после `pub mod auth;` в `src/lib.rs`)
Expected: ошибки компиляции `cannot find type 'Access'`.

- [ ] **Step 2: Реализовать**

Добавить в начало `src/auth.rs`:

```rust
//! Who may see the scenes, how hard one may guess the password, and the
//! one-time code that creates the admin on a fresh install.

use crate::crypto;
use crate::store::Store;
use axum::http::{HeaderMap, HeaderName};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

pub const ACCESS_KEY: &str = "access";
pub const SETUP_CODE_KEY: &str = "setup_code";
const SETUP_TTL_MS: i64 = 24 * 3600 * 1000;
const WINDOW_MS: i64 = 15 * 60 * 1000;
const MAX_FAILURES: usize = 5;
const MAX_TRACKED: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Access {
    /// Anyone who can reach lorvaq sees the scenes.
    Open,
    /// A reverse proxy in front signs viewers in and passes `header`; only
    /// requests coming from `trusted` addresses count.
    Proxy { header: String, trusted: Vec<String> },
    /// Only the signed-in admin.
    #[default]
    Admin,
}

fn parse_net(s: &str) -> Option<IpNet> {
    let s = s.trim();
    s.parse::<IpNet>().ok().or_else(|| s.parse::<IpAddr>().ok().map(IpNet::from))
}

impl Access {
    pub fn validate(&self) -> Result<(), String> {
        if let Access::Proxy { header, trusted } = self {
            if header.is_empty() || HeaderName::from_bytes(header.as_bytes()).is_err() {
                return Err(format!("{header:?} is not a valid header name"));
            }
            if trusted.is_empty() {
                return Err("list at least one trusted proxy address".into());
            }
            if let Some(bad) = trusted.iter().find(|t| parse_net(t).is_none()) {
                return Err(format!("{bad:?} is neither an address nor a network"));
            }
        }
        Ok(())
    }

    pub fn viewer_allowed(&self, peer: Option<IpAddr>, headers: &HeaderMap, admin: bool) -> bool {
        if admin {
            return true;
        }
        match self {
            Access::Open => true,
            Access::Admin => false,
            Access::Proxy { header, trusted } => {
                let from_proxy = peer.is_some_and(|ip| trusted.iter().filter_map(|t| parse_net(t)).any(|n| n.contains(&ip)));
                from_proxy && headers.get(header.as_str()).is_some_and(|v| !v.as_bytes().is_empty())
            }
        }
    }
}

#[derive(Default)]
pub struct LoginLimiter {
    failures: HashMap<IpAddr, Vec<i64>>,
}

impl LoginLimiter {
    pub fn blocked(&mut self, ip: IpAddr, now_ms: i64) -> bool {
        match self.failures.get_mut(&ip) {
            Some(times) => {
                times.retain(|t| now_ms - t < WINDOW_MS);
                times.len() >= MAX_FAILURES
            }
            None => false,
        }
    }

    pub fn fail(&mut self, ip: IpAddr, now_ms: i64) {
        if self.failures.len() >= MAX_TRACKED {
            self.failures.retain(|_, times| times.last().is_some_and(|t| now_ms - t < WINDOW_MS));
        }
        self.failures.entry(ip).or_default().push(now_ms);
    }

    pub fn reset(&mut self, ip: IpAddr) {
        self.failures.remove(&ip);
    }
}

#[derive(Serialize, Deserialize)]
struct SetupCode {
    hash: String,
    expires_ms: i64,
}

fn normalize(code: &str) -> String {
    code.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase()
}

/// A new code replaces any older one.
pub async fn issue_setup_code(store: &Store, now_ms: i64) -> anyhow::Result<String> {
    let code = crypto::setup_code();
    let saved = SetupCode { hash: crypto::token_hash(&normalize(&code)), expires_ms: now_ms + SETUP_TTL_MS };
    store.put_setting(SETUP_CODE_KEY, &saved).await?;
    Ok(code)
}

pub async fn check_setup_code(store: &Store, code: &str, now_ms: i64) -> anyhow::Result<bool> {
    Ok(match store.setting::<SetupCode>(SETUP_CODE_KEY).await? {
        Some(saved) => saved.expires_ms > now_ms && saved.hash == crypto::token_hash(&normalize(code)),
        None => false,
    })
}

/// On a fresh install there is no admin: print a one-time code to create one.
pub async fn announce_setup_code(store: &Store, now_ms: i64) -> anyhow::Result<()> {
    if store.admin_hash().await?.is_none() {
        let code = issue_setup_code(store, now_ms).await?;
        tracing::warn!("no admin yet: open lorvaq and enter the one-time setup code {code} (valid for 24 hours)");
    }
    Ok(())
}
```

- [ ] **Step 3: Запустить — тесты проходят**

Run: `cargo test auth:: && cargo test --test store && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `5 passed` в `auth::`, `8 passed` в `store`.

- [ ] **Step 4: Commit**

```bash
git add src/ tests/
git commit -m "feat(auth): viewer access modes, login limiter, one-time setup code

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 6: HTTP — вход и настройки

**Files:**
- Replace: `src/http/mod.rs`
- Create: `src/http/guard.rs`, `src/http/account.rs`, `src/http/admin.rs`, `tests/http_live.rs`

**Interfaces:**
- Consumes: всё из задач 1–5.
- Produces:
  - `http::{AppState { scenes: Vec<SceneHandle>, live: Option<Live> }, AppState::demo(Vec<SceneHandle>), Live { store, secrets, cookie_secure, access: RwLock<Access>, limiter: Mutex<LoginLimiter>, history: Vec<(&'static str, Arc<Mutex<HistoryStatus>>)> }, router(Arc<AppState>) -> Router}`
  - маршруты (контракт для фронта, задача 8):
    - публичные: `GET /healthz` → `{"status","db": bool|null,"scenes"}`; `GET /api/v1/me` → `{"demo","setup_needed","admin","viewer"}`; `POST /api/v1/setup {code,password}`; `POST /api/v1/login {password}`; `POST /api/v1/logout`
    - зрителя: `/api/v1/scenes`, `/api/v1/scenes/{id}/snapshot`, `/api/v1/scenes/{id}/events` — 401 `{"error"}`, если режим не пускает
    - администратора (401 без сессии, 404 в demo): `GET|PUT /api/v1/admin/access` (тело — `Access`), `GET /api/v1/admin/history` → `{"retention_days","scenes":[{"id","queued","dropped","written","last_error"}]}`, `PUT /api/v1/admin/history {retention_days}`, `GET|PUT /api/v1/admin/scenes/{id}/settings`, `POST /api/v1/admin/password {current,new}`
  - коды: 400 неверный ввод, 401 нет входа / неверный пароль, 403 неверный код настройки или текущий пароль, 404 нет сцены или demo, 409 администратор уже есть / ещё нет, 429 слишком много попыток, 503 база недоступна.

- [ ] **Step 1: Написать падающие интеграционные тесты**

`tests/http_live.rs`:

```rust
mod common;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::StatusCode;
use axum::Router;
use common::call;
use lorvaq::auth::{self, Access, LoginLimiter};
use lorvaq::crypto::{self, SecretBox};
use lorvaq::http::{router, AppState, Live};
use lorvaq::scene;
use lorvaq::scenes::ci::{settings::CiSettings, CiScene};
use lorvaq::store::Store;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const PASSWORD: &str = "correct horse battery";

async fn app(access: Access) -> (Router, Store) {
    let store = common::store().await;
    let (ci, _inputs) = scene::spawn(CiScene::new(CiSettings::default()).unwrap(), Duration::from_secs(1));
    let live = Live {
        store: store.clone(),
        secrets: SecretBox::from_base64(&SecretBox::generate_key()).unwrap(),
        cookie_secure: false,
        access: RwLock::new(access),
        limiter: Mutex::new(LoginLimiter::default()),
        history: vec![],
    };
    let router = router(Arc::new(AppState { scenes: vec![ci], live: Some(live) })).layer(MockConnectInfo(SocketAddr::from(([10, 1, 2, 3], 4000))));
    (router, store)
}

async fn signed_in(access: Access) -> (Router, Store, String) {
    let (r, store) = app(access).await;
    store.set_admin_hash(&crypto::hash_password(PASSWORD), 0).await.unwrap();
    let (status, _, cookie) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": PASSWORD })), &[]).await;
    assert_eq!(status, StatusCode::OK);
    (r, store, cookie.expect("a session cookie"))
}

#[tokio::test]
async fn first_run_setup_creates_the_admin() {
    let (r, store) = app(Access::Admin).await;
    let (_, me, _) = call(&r, "GET", "/api/v1/me", None, None, &[]).await;
    assert_eq!(me, json!({ "demo": false, "setup_needed": true, "admin": false, "viewer": false }));
    let code = auth::issue_setup_code(&store, lorvaq::scene::now_ms()).await.unwrap();

    let (status, body, _) = call(&r, "POST", "/api/v1/setup", None, Some(json!({ "code": "WRONG", "password": PASSWORD })), &[]).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _, _) = call(&r, "POST", "/api/v1/setup", None, Some(json!({ "code": code, "password": "short" })), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, cookie) = call(&r, "POST", "/api/v1/setup", None, Some(json!({ "code": code, "password": PASSWORD })), &[]).await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("setup signs the admin in");
    assert!(cookie.starts_with("lorvaq_session="));

    let (_, me, _) = call(&r, "GET", "/api/v1/me", Some(&cookie), None, &[]).await;
    assert_eq!(me, json!({ "demo": false, "setup_needed": false, "admin": true, "viewer": true }));
    let (status, _, _) = call(&r, "POST", "/api/v1/setup", None, Some(json!({ "code": code, "password": PASSWORD })), &[]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, access, _) = call(&r, "GET", "/api/v1/admin/access", Some(&cookie), None, &[]).await;
    assert_eq!((status, access), (StatusCode::OK, json!({ "mode": "admin" })));
}

#[tokio::test]
async fn login_logout_and_lockout() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let (status, _, _) = call(&r, "GET", "/api/v1/admin/access", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, cleared) = call(&r, "POST", "/api/v1/logout", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleared.as_deref(), Some("lorvaq_session="));
    let (status, _, _) = call(&r, "GET", "/api/v1/admin/access", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the session is gone");

    for _ in 0..5 {
        let (status, _, _) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": "wrong wrong wrong" })), &[]).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, _) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": PASSWORD })), &[]).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "blocked even with the right password");
    drop(store);
}

#[tokio::test]
async fn viewer_modes_guard_the_scenes() {
    let (r, _) = app(Access::Admin).await;
    assert_eq!(call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[]).await.0, StatusCode::UNAUTHORIZED);
    let (r, _) = app(Access::Open).await;
    assert_eq!(call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[]).await.0, StatusCode::OK);
    let proxy = |net: &str| Access::Proxy { header: "x-forwarded-user".into(), trusted: vec![net.into()] };
    let (r, _) = app(proxy("10.0.0.0/8")).await;
    assert_eq!(call(&r, "GET", "/api/v1/scenes", None, None, &[]).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&r, "GET", "/api/v1/scenes", None, None, &[("x-forwarded-user", "dana")]).await.0, StatusCode::OK);
    let (r, _) = app(proxy("192.168.0.0/16")).await;
    assert_eq!(call(&r, "GET", "/api/v1/scenes", None, None, &[("x-forwarded-user", "dana")]).await.0, StatusCode::UNAUTHORIZED, "not our proxy");
}

#[tokio::test]
async fn admin_routes_need_a_session() {
    let (r, _) = app(Access::Open).await;
    for (method, uri) in [("GET", "/api/v1/admin/access"), ("GET", "/api/v1/admin/history"), ("GET", "/api/v1/admin/scenes/ci/settings")] {
        assert_eq!(call(&r, method, uri, None, None, &[]).await.0, StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn access_and_history_settings_are_validated_and_kept() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let bad = json!({ "mode": "proxy", "header": "x-forwarded-user", "trusted": [] });
    let (status, body, _) = call(&r, "PUT", "/api/v1/admin/access", Some(&cookie), Some(bad), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("trusted"), "{body}");
    let (status, _, _) = call(&r, "PUT", "/api/v1/admin/access", Some(&cookie), Some(json!({ "mode": "open" })), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store.setting::<Access>(auth::ACCESS_KEY).await.unwrap(), Some(Access::Open));
    assert_eq!(call(&r, "GET", "/api/v1/scenes", None, None, &[]).await.0, StatusCode::OK, "applies at once");

    let (status, _, _) = call(&r, "PUT", "/api/v1/admin/history", Some(&cookie), Some(json!({ "retention_days": 0 })), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(&r, "PUT", "/api/v1/admin/history", Some(&cookie), Some(json!({ "retention_days": 30 })), &[]).await;
    assert_eq!(status, StatusCode::OK);
    let (_, history, _) = call(&r, "GET", "/api/v1/admin/history", Some(&cookie), None, &[]).await;
    assert_eq!(history["retention_days"], json!(30));
}

#[tokio::test]
async fn scene_settings_apply_live_and_bad_ones_change_nothing() {
    let (r, store, cookie) = signed_in(Access::Admin).await;
    let (status, current, _) = call(&r, "GET", "/api/v1/admin/scenes/ci/settings", Some(&cookie), None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["max_projects"], json!(8));

    let mut next = current.clone();
    next["exits"] = json!([{ "label": "prod", "stage": "deploy" }]);
    let (status, applied, _) = call(&r, "PUT", "/api/v1/admin/scenes/ci/settings", Some(&cookie), Some(next), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(applied["exits"][0]["label"], json!("prod"));
    assert_eq!(store.scene_settings("ci").await.unwrap().unwrap()["exits"][0]["label"], json!("prod"));

    let mut bad = applied.clone();
    bad["exits"] = json!([{ "label": "x", "stage": "(" }]);
    let (status, body, _) = call(&r, "PUT", "/api/v1/admin/scenes/ci/settings", Some(&cookie), Some(bad), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(store.scene_settings("ci").await.unwrap().unwrap()["exits"][0]["label"], json!("prod"), "nothing stored");
    assert_eq!(call(&r, "GET", "/api/v1/admin/scenes/nope/settings", Some(&cookie), None, &[]).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_down_database_answers_503_and_me_still_works() {
    let (r, store, _) = signed_in(Access::Open).await;
    store.close().await;
    let (status, body, _) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": PASSWORD })), &[]).await;
    assert_eq!((status, body), (StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "the database is unavailable" })));
    let (status, me, _) = call(&r, "GET", "/api/v1/me", None, None, &[]).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(call(&r, "GET", "/api/v1/scenes/ci/snapshot", None, None, &[]).await.0, StatusCode::OK, "the live scene does not need the database");
}

#[tokio::test]
async fn changing_the_password_signs_other_sessions_out() {
    let (r, _, mine) = signed_in(Access::Admin).await;
    let (_, _, other) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": PASSWORD })), &[]).await;
    let other = other.unwrap();
    let wrong = json!({ "current": "not it at all", "new": "a brand new password" });
    assert_eq!(call(&r, "POST", "/api/v1/admin/password", Some(&mine), Some(wrong), &[]).await.0, StatusCode::FORBIDDEN);
    let good = json!({ "current": PASSWORD, "new": "a brand new password" });
    assert_eq!(call(&r, "POST", "/api/v1/admin/password", Some(&mine), Some(good), &[]).await.0, StatusCode::OK);
    assert_eq!(call(&r, "GET", "/api/v1/admin/access", Some(&mine), None, &[]).await.0, StatusCode::OK, "my session stays");
    assert_eq!(call(&r, "GET", "/api/v1/admin/access", Some(&other), None, &[]).await.0, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(&r, "POST", "/api/v1/login", None, Some(json!({ "password": "a brand new password" })), &[]).await;
    assert_eq!(status, StatusCode::OK);
}
```

Run: `cargo test --test http_live`
Expected: ошибки компиляции `cannot find struct 'Live' in module 'lorvaq::http'`.

- [ ] **Step 2: Заменить `src/http/mod.rs` целиком**

```rust
//! HTTP: scenes (behind the viewer guard), sign-in, admin settings (behind the
//! admin guard), and the built-in UI.

mod account;
mod admin;
mod guard;
mod static_files;

use crate::auth::{Access, LoginLimiter};
use crate::crypto::SecretBox;
use crate::history::HistoryStatus;
use crate::scene::SceneHandle;
use crate::store::Store;
use crate::world::bus::Gap;
use crate::world::Event;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;

pub struct AppState {
    pub scenes: Vec<SceneHandle>,
    /// None in demo mode: no database, no admin, everything open.
    pub live: Option<Live>,
}

pub struct Live {
    pub store: Store,
    /// Encrypts source credentials (used from plan 3 on).
    pub secrets: SecretBox,
    pub cookie_secure: bool,
    pub access: RwLock<Access>,
    pub limiter: Mutex<LoginLimiter>,
    pub history: Vec<(&'static str, Arc<Mutex<HistoryStatus>>)>,
}

impl AppState {
    pub fn demo(scenes: Vec<SceneHandle>) -> Self {
        Self { scenes, live: None }
    }

    fn scene(&self, id: &str) -> Option<&SceneHandle> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let viewer = Router::new()
        .route("/api/v1/scenes", get(list_scenes))
        .route("/api/v1/scenes/{id}/snapshot", get(snapshot))
        .route("/api/v1/scenes/{id}/events", get(events))
        .route_layer(middleware::from_fn_with_state(state.clone(), guard::viewer));
    let admin = Router::new()
        .route("/api/v1/admin/access", get(admin::get_access).put(admin::put_access))
        .route("/api/v1/admin/history", get(admin::get_history).put(admin::put_history))
        .route("/api/v1/admin/scenes/{id}/settings", get(admin::get_scene).put(admin::put_scene))
        .route("/api/v1/admin/password", post(account::change_password))
        .route_layer(middleware::from_fn_with_state(state.clone(), guard::admin));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/me", get(account::me))
        .route("/api/v1/setup", post(account::setup))
        .route("/api/v1/login", post(account::login))
        .route("/api/v1/logout", post(account::logout))
        .merge(viewer)
        .merge(admin)
        .fallback(static_files::serve)
        .with_state(state)
}

async fn healthz(State(s): State<Arc<AppState>>) -> Json<Value> {
    let scenes: Vec<Value> = s.scenes.iter().map(|h| json!({ "id": h.id, "stale": h.snapshot().stale })).collect();
    let db = match &s.live {
        Some(live) => json!(live.store.ping().await),
        None => Value::Null,
    };
    Json(json!({ "status": "ok", "db": db, "scenes": scenes }))
}

async fn list_scenes(State(s): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(s.scenes.iter().map(|h| json!({ "id": h.id, "title": h.title })).collect::<Vec<_>>()))
}

async fn snapshot(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match s.scene(&id) {
        Some(h) => Json(h.snapshot()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct EventsQuery {
    since: Option<u64>,
}

/// Backlog after `since` (or `Last-Event-ID`), then live events. A gap is 410:
/// the client takes a fresh snapshot. A lagging client is cut off and reconnects.
async fn events(State(s): State<Arc<AppState>>, Path(id): Path<String>, Query(q): Query<EventsQuery>, headers: HeaderMap) -> Response {
    let Some(h) = s.scene(&id) else { return StatusCode::NOT_FOUND.into_response() };
    let from_header = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    // Subscribe before reading the backlog, so nothing falls between the two.
    let rx = h.bus.subscribe();
    let after = from_header.or(q.since).unwrap_or_else(|| h.bus.last_seq());
    let backlog = match h.bus.since(after) {
        Ok(events) => events,
        Err(Gap) => return StatusCode::GONE.into_response(),
    };
    let sent_up_to = backlog.last().map_or(after, |e| e.seq);
    let live = BroadcastStream::new(rx)
        .take_while(|r| futures::future::ready(r.is_ok()))
        .filter_map(|r| futures::future::ready(r.ok()))
        .filter(move |e| futures::future::ready(e.seq > sent_up_to));
    let stream = futures::stream::iter(backlog).chain(live).map(to_sse);
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

fn to_sse(ev: Event) -> Result<SseEvent, Infallible> {
    Ok(SseEvent::default().id(ev.seq.to_string()).json_data(&ev).expect("events always serialize"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Attrs, Change, Place};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn app(cap: usize) -> (Router, SceneHandle) {
        let h = SceneHandle::new("test", "Test", cap);
        (router(Arc::new(AppState::demo(vec![h.clone()]))), h)
    }

    fn zone(id: &str) -> Change {
        Change::UpsertPlace { place: Place { id: id.into(), kind: "zone".into(), parent: None, label: id.into(), capacity: None, attrs: Attrs::new() } }
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
        let res = app.clone().oneshot(Request::get(uri).body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into())
    }

    /// Event ids in an SSE text, in order.
    fn ids(text: &str) -> Vec<u64> {
        text.lines().filter_map(|l| l.strip_prefix("id:")?.trim().parse().ok()).collect()
    }

    /// Reads SSE frames until `want` events arrived; fails after 2 s of silence.
    async fn read_events(res: Response, want: usize) -> String {
        assert_eq!(res.status(), StatusCode::OK);
        let mut body = res.into_body();
        let mut text = String::new();
        while ids(&text).len() < want {
            let frame = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.expect("an event within 2s").expect("stream open").unwrap();
            if let Ok(data) = frame.into_data() {
                text.push_str(&String::from_utf8_lossy(&data));
            }
        }
        text
    }

    async fn open(app: &Router, uri: &str, last_event_id: Option<&str>) -> Response {
        let mut req = Request::get(uri);
        if let Some(id) = last_event_id {
            req = req.header("last-event-id", id);
        }
        app.clone().oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn lists_scenes_and_health() {
        let (app, _) = app(10);
        let (status, body) = get(&app, "/api/v1/scenes").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!([{"id": "test", "title": "Test"}]));
        let (status, body) = get(&app, "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        let health: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((health["status"].as_str(), health["db"].clone()), (Some("ok"), Value::Null));
    }

    #[tokio::test]
    async fn demo_mode_is_open_and_has_no_admin() {
        let (app, _) = app(10);
        let (status, body) = get(&app, "/api/v1/me").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!({"demo": true, "setup_needed": false, "admin": false, "viewer": true}));
        assert_eq!(get(&app, "/api/v1/admin/access").await.0, StatusCode::NOT_FOUND);
        let req = Request::post("/api/v1/login").header("content-type", "application/json").body(Body::from(r#"{"password":"x"}"#)).unwrap();
        assert_eq!(app.clone().oneshot(req).await.unwrap().status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn snapshot_of_known_and_unknown_scenes() {
        let (app, h) = app(10);
        h.commit(vec![zone("a"), zone("b")], 1);
        let (status, body) = get(&app, "/api/v1/scenes/test/snapshot").await;
        assert_eq!(status, StatusCode::OK);
        let snap: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((snap["seq"].as_u64(), snap["places"].as_array().map(Vec::len)), (Some(2), Some(2)));
        assert_eq!(get(&app, "/api/v1/scenes/nope/snapshot").await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn events_replay_the_backlog() {
        let (app, h) = app(10);
        h.commit(vec![zone("a"), zone("b")], 1);
        let text = read_events(open(&app, "/api/v1/scenes/test/events?since=0", None).await, 2).await;
        assert_eq!(ids(&text), vec![1, 2], "{text}");
        assert!(text.contains(r#""op":"upsert_place""#), "{text}");
    }

    #[tokio::test]
    async fn events_stream_live_changes_without_duplicates() {
        let (app, h) = app(10);
        h.commit(vec![zone("a")], 1);
        let res = open(&app, "/api/v1/scenes/test/events?since=0", None).await;
        h.commit(vec![zone("b")], 2);
        let text = read_events(res, 2).await;
        assert_eq!(ids(&text), vec![1, 2], "each event exactly once: {text}");
    }

    #[tokio::test]
    async fn last_event_id_wins_over_the_query() {
        let (app, h) = app(10);
        h.commit(vec![zone("a"), zone("b")], 1);
        let text = read_events(open(&app, "/api/v1/scenes/test/events?since=0", Some("1")).await, 1).await;
        assert_eq!(ids(&text), vec![2], "{text}");
    }

    #[tokio::test]
    async fn gaps_and_future_ids_are_gone() {
        let (app, h) = app(2);
        h.commit(vec![zone("a"), zone("b"), zone("c"), zone("d")], 1);
        assert_eq!(open(&app, "/api/v1/scenes/test/events?since=1", None).await.status(), StatusCode::GONE);
        assert_eq!(open(&app, "/api/v1/scenes/test/events?since=9", None).await.status(), StatusCode::GONE);
        assert_eq!(open(&app, "/api/v1/scenes/test/events?since=2", None).await.status(), StatusCode::OK);
        assert_eq!(open(&app, "/api/v1/scenes/nope/events", None).await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ui_is_served_and_unknown_api_is_404() {
        let (app, _) = app(10);
        let (status, body) = get(&app, "/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("lorvaq"), "{body}");
        assert_eq!(get(&app, "/api/v1/nothing").await.0, StatusCode::NOT_FOUND);
    }
}
```

- [ ] **Step 3: Охранники, вход и администрирование**

`src/http/guard.rs`:

```rust
//! Guards in front of the scene and admin routes.

use super::AppState;
use crate::crypto;
use crate::scene::{now_ms, LOCK};
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde_json::json;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

pub(super) const SESSION_COOKIE: &str = "lorvaq_session";

/// The address the request came from (None when the server runs without connect info, as in tests).
pub(super) struct Peer(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Peer(parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_canonical())))
    }
}

pub(super) fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub(super) fn db_error(e: anyhow::Error) -> Response {
    tracing::error!("database: {e:#}");
    json_error(StatusCode::SERVICE_UNAVAILABLE, "the database is unavailable")
}

/// Is there a valid admin session cookie? A database error counts as "no".
pub(super) async fn has_session(state: &AppState, jar: &CookieJar) -> bool {
    let (Some(live), Some(cookie)) = (&state.live, jar.get(SESSION_COOKIE)) else { return false };
    live.store.session_valid(&crypto::token_hash(cookie.value()), now_ms()).await.unwrap_or_else(|e| {
        tracing::warn!("session check failed: {e:#}");
        false
    })
}

pub(super) async fn viewer(State(s): State<Arc<AppState>>, Peer(peer): Peer, jar: CookieJar, req: Request, next: Next) -> Response {
    let Some(live) = &s.live else { return next.run(req).await };
    let access = live.access.read().expect(LOCK).clone();
    let admin = has_session(&s, &jar).await;
    if access.viewer_allowed(peer, req.headers(), admin) {
        next.run(req).await
    } else {
        json_error(StatusCode::UNAUTHORIZED, "sign in to see the scenes")
    }
}

pub(super) async fn admin(State(s): State<Arc<AppState>>, jar: CookieJar, req: Request, next: Next) -> Response {
    if s.live.is_none() {
        return json_error(StatusCode::NOT_FOUND, "settings are not available in demo mode");
    }
    if has_session(&s, &jar).await {
        next.run(req).await
    } else {
        json_error(StatusCode::UNAUTHORIZED, "sign in as the admin")
    }
}
```

`src/http/account.rs`:

```rust
//! Who am I, first-run setup, sign in and out, password change.

use super::guard::{db_error, has_session, json_error, Peer, SESSION_COOKIE};
use super::{AppState, Live};
use crate::auth;
use crate::crypto;
use crate::scene::{now_ms, LOCK};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;

pub const MIN_PASSWORD: usize = 12;
const SESSION_TTL_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Deserialize)]
pub(super) struct SetupBody {
    code: String,
    password: String,
}

#[derive(Deserialize)]
pub(super) struct LoginBody {
    password: String,
}

#[derive(Deserialize)]
pub(super) struct PasswordBody {
    current: String,
    new: String,
}

fn short_password() -> Response {
    json_error(StatusCode::BAD_REQUEST, &format!("the password needs at least {MIN_PASSWORD} characters"))
}

async fn hash_blocking(password: String) -> String {
    tokio::task::spawn_blocking(move || crypto::hash_password(&password)).await.expect("hashing does not panic")
}

async fn verify_blocking(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || crypto::verify_password(&password, &hash)).await.unwrap_or(false)
}

fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token)).http_only(true).secure(secure).same_site(SameSite::Lax).path("/").max_age(time::Duration::days(30)).build()
}

async fn start_session(live: &Live, jar: CookieJar, now: i64) -> Response {
    let token = crypto::random_token();
    if let Err(e) = live.store.create_session(&crypto::token_hash(&token), now, SESSION_TTL_MS).await {
        return db_error(e);
    }
    (jar.add(session_cookie(token, live.cookie_secure)), Json(json!({ "ok": true }))).into_response()
}

fn limiter_key(peer: Option<IpAddr>) -> IpAddr {
    peer.unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

pub(super) async fn me(State(s): State<Arc<AppState>>, Peer(peer): Peer, jar: CookieJar, headers: HeaderMap) -> Json<Value> {
    let Some(live) = &s.live else {
        return Json(json!({ "demo": true, "setup_needed": false, "admin": false, "viewer": true }));
    };
    let admin = has_session(&s, &jar).await;
    let setup_needed = matches!(live.store.admin_hash().await, Ok(None));
    let access = live.access.read().expect(LOCK).clone();
    let viewer = access.viewer_allowed(peer, &headers, admin);
    Json(json!({ "demo": false, "setup_needed": setup_needed, "admin": admin, "viewer": viewer }))
}

pub(super) async fn setup(State(s): State<Arc<AppState>>, Peer(peer): Peer, jar: CookieJar, Json(body): Json<SetupBody>) -> Response {
    let Some(live) = &s.live else { return json_error(StatusCode::NOT_FOUND, "demo mode has no admin") };
    let (ip, now) = (limiter_key(peer), now_ms());
    if live.limiter.lock().expect(LOCK).blocked(ip, now) {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again in 15 minutes");
    }
    match live.store.admin_hash().await {
        Ok(Some(_)) => return json_error(StatusCode::CONFLICT, "the admin already exists"),
        Ok(None) => {}
        Err(e) => return db_error(e),
    }
    if body.password.chars().count() < MIN_PASSWORD {
        return short_password();
    }
    match auth::check_setup_code(&live.store, &body.code, now).await {
        Ok(true) => {}
        Ok(false) => {
            live.limiter.lock().expect(LOCK).fail(ip, now);
            return json_error(StatusCode::FORBIDDEN, "wrong or expired setup code");
        }
        Err(e) => return db_error(e),
    }
    if let Err(e) = live.store.set_admin_hash(&hash_blocking(body.password).await, now).await {
        return db_error(e);
    }
    if let Err(e) = live.store.delete_setting(auth::SETUP_CODE_KEY).await {
        tracing::warn!("the used setup code stays until it expires: {e:#}");
    }
    start_session(live, jar, now).await
}

pub(super) async fn login(State(s): State<Arc<AppState>>, Peer(peer): Peer, jar: CookieJar, Json(body): Json<LoginBody>) -> Response {
    let Some(live) = &s.live else { return json_error(StatusCode::NOT_FOUND, "demo mode has no admin") };
    let (ip, now) = (limiter_key(peer), now_ms());
    if live.limiter.lock().expect(LOCK).blocked(ip, now) {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "too many attempts; try again in 15 minutes");
    }
    let hash = match live.store.admin_hash().await {
        Ok(Some(hash)) => hash,
        Ok(None) => return json_error(StatusCode::CONFLICT, "no admin yet: finish the setup first"),
        Err(e) => return db_error(e),
    };
    if !verify_blocking(body.password, hash).await {
        live.limiter.lock().expect(LOCK).fail(ip, now);
        return json_error(StatusCode::UNAUTHORIZED, "wrong password");
    }
    live.limiter.lock().expect(LOCK).reset(ip);
    start_session(live, jar, now).await
}

pub(super) async fn logout(State(s): State<Arc<AppState>>, jar: CookieJar) -> Response {
    if let (Some(live), Some(cookie)) = (&s.live, jar.get(SESSION_COOKIE)) {
        if let Err(e) = live.store.delete_session(&crypto::token_hash(cookie.value())).await {
            return db_error(e);
        }
    }
    let mut gone = Cookie::from(SESSION_COOKIE);
    gone.set_path("/");
    (jar.remove(gone), Json(json!({ "ok": true }))).into_response()
}

/// Behind the admin guard.
pub(super) async fn change_password(State(s): State<Arc<AppState>>, jar: CookieJar, Json(body): Json<PasswordBody>) -> Response {
    let live = s.live.as_ref().expect("the admin guard lets only live requests in");
    if body.new.chars().count() < MIN_PASSWORD {
        return short_password();
    }
    let hash = match live.store.admin_hash().await {
        Ok(Some(hash)) => hash,
        Ok(None) => return json_error(StatusCode::CONFLICT, "no admin yet"),
        Err(e) => return db_error(e),
    };
    if !verify_blocking(body.current, hash).await {
        return json_error(StatusCode::FORBIDDEN, "the current password is wrong");
    }
    if let Err(e) = live.store.set_admin_hash(&hash_blocking(body.new).await, now_ms()).await {
        return db_error(e);
    }
    let mine = jar.get(SESSION_COOKIE).map(|c| crypto::token_hash(c.value())).expect("the admin guard checked the cookie");
    if let Err(e) = live.store.delete_other_sessions(&mine).await {
        return db_error(e);
    }
    Json(json!({ "ok": true })).into_response()
}
```

`src/http/admin.rs`:

```rust
//! Admin settings: viewer access, history retention, scene settings.

use super::guard::{db_error, json_error};
use super::AppState;
use crate::auth::{self, Access};
use crate::history::{HistorySettings, HISTORY_KEY};
use crate::scene::{now_ms, LOCK};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

fn live(s: &AppState) -> &super::Live {
    s.live.as_ref().expect("the admin guard lets only live requests in")
}

pub(super) async fn get_access(State(s): State<Arc<AppState>>) -> Json<Access> {
    Json(live(&s).access.read().expect(LOCK).clone())
}

pub(super) async fn put_access(State(s): State<Arc<AppState>>, Json(access): Json<Access>) -> Response {
    if let Err(message) = access.validate() {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }
    let live = live(&s);
    if let Err(e) = live.store.put_setting(auth::ACCESS_KEY, &access).await {
        return db_error(e);
    }
    *live.access.write().expect(LOCK) = access.clone();
    Json(access).into_response()
}

pub(super) async fn get_history(State(s): State<Arc<AppState>>) -> Response {
    let live = live(&s);
    let settings = match live.store.setting::<HistorySettings>(HISTORY_KEY).await {
        Ok(found) => found.unwrap_or_default(),
        Err(e) => return db_error(e),
    };
    let scenes: Vec<Value> = live
        .history
        .iter()
        .map(|(id, status)| {
            let st = status.lock().expect(LOCK).clone();
            json!({ "id": id, "queued": st.queued, "dropped": st.dropped, "written": st.written, "last_error": st.last_error })
        })
        .collect();
    Json(json!({ "retention_days": settings.retention_days, "scenes": scenes })).into_response()
}

pub(super) async fn put_history(State(s): State<Arc<AppState>>, Json(settings): Json<HistorySettings>) -> Response {
    if let Err(message) = settings.validate() {
        return json_error(StatusCode::BAD_REQUEST, &message);
    }
    if let Err(e) = live(&s).store.put_setting(HISTORY_KEY, &settings).await {
        return db_error(e);
    }
    Json(settings).into_response()
}

pub(super) async fn get_scene(State(s): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match s.scene(&id) {
        Some(h) => Json(h.settings()).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "no such scene"),
    }
}

/// Applies the settings in the running scene first; only what it accepted is stored.
pub(super) async fn put_scene(State(s): State<Arc<AppState>>, Path(id): Path<String>, Json(value): Json<Value>) -> Response {
    let Some(h) = s.scene(&id) else { return json_error(StatusCode::NOT_FOUND, "no such scene") };
    match h.configure(value).await {
        Ok(applied) => match live(&s).store.put_scene_settings(&id, &applied, now_ms()).await {
            Ok(()) => Json(applied).into_response(),
            Err(e) => db_error(e),
        },
        Err(e) => json_error(StatusCode::BAD_REQUEST, &format!("{e:#}")),
    }
}
```

- [ ] **Step 4: Поправить вызов `serve` в `app.rs`, чтобы всё собиралось**

В `src/app.rs` функция `serve` пока принимает сцены; заменить строку `let app = router(Arc::new(AppState { scenes }));` на `let app = router(Arc::new(AppState::demo(scenes)));` (задача 7 перепишет `app.rs` целиком).

- [ ] **Step 5: Запустить — всё проходит**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: все тесты зелёные, в т.ч. `http_live` — `8 passed`, `http::` — `8 passed`.

- [ ] **Step 6: Commit**

```bash
git add src/ tests/
git commit -m "feat(http): first-run setup, admin sign-in, viewer and admin guards, settings API

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 7: Живой режим сервера и сброс пароля

**Files:**
- Replace: `src/app.rs`, `src/main.rs`
- Create: `tests/live_smoke.rs`

**Interfaces:**
- Consumes: всё из задач 1–6.
- Produces: `app::{demo_scenes(), live_state(Store, SecretBox, Option<&str>) -> anyhow::Result<AppState>, run_server(ServerArgs), reset_password(DbArgs), serve(SocketAddr, AppState)}`.

- [ ] **Step 1: Написать падающий тест**

`tests/live_smoke.rs`:

```rust
mod common;

use axum::http::StatusCode;
use lorvaq::app;
use lorvaq::crypto::SecretBox;
use lorvaq::http::router;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn live_server_writes_history_and_asks_for_setup() {
    let store = common::store().await;
    store.put_scene_settings("ci", &json!({ "exits": [{ "label": "prod", "stage": "deploy" }] }), 0).await.unwrap();
    let state = app::live_state(store.clone(), SecretBox::from_base64(&SecretBox::generate_key()).unwrap(), None).await.unwrap();
    assert_eq!(state.scenes[0].settings()["exits"][0]["label"], json!("prod"), "stored settings are loaded");
    let r = router(Arc::new(state));

    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(store.event_count("ci").await.unwrap() > 0, "the scene's first events reached the history");
    let (status, me, _) = common::call(&r, "GET", "/api/v1/me", None, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me, json!({ "demo": false, "setup_needed": true, "admin": false, "viewer": false }));
    assert!(store.setting::<serde_json::Value>(lorvaq::auth::SETUP_CODE_KEY).await.unwrap().is_some(), "a setup code was issued");
    let (_, health, _) = common::call(&r, "GET", "/healthz", None, None, &[]).await;
    assert_eq!(health["db"], json!(true));
}

#[tokio::test]
async fn broken_stored_settings_fall_back_to_defaults() {
    let store = common::store().await;
    store.put_scene_settings("ci", &json!({ "exits": [{ "label": "x", "stage": "(" }] }), 0).await.unwrap();
    let state = app::live_state(store, SecretBox::from_base64(&SecretBox::generate_key()).unwrap(), None).await.unwrap();
    assert_eq!(state.scenes[0].settings()["exits"], json!([]));
}
```

Run: `cargo test --test live_smoke`
Expected: ошибка компиляции `cannot find function 'live_state' in module 'app'`.

- [ ] **Step 2: Заменить `src/app.rs` целиком**

```rust
//! Wires scenes, sources, storage and HTTP into a running server.

use crate::auth::{self, Access, LoginLimiter};
use crate::cli::{DbArgs, ServerArgs};
use crate::crypto::SecretBox;
use crate::history::{self, HistoryStatus};
use crate::http::{router, AppState, Live};
use crate::scene::{self, now_ms, SceneHandle};
use crate::scenes::ci::settings::CiSettings;
use crate::scenes::ci::CiScene;
use crate::store::Store;
use anyhow::Context;
use futures::FutureExt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Event streams never end on their own; after a stop signal they get this long.
const STREAM_GRACE: Duration = Duration::from_secs(2);

/// The CI scene fed by the simulator. Needs a tokio runtime.
pub fn demo_scenes() -> anyhow::Result<Vec<SceneHandle>> {
    let ci = CiScene::new(crate::demo::settings())?;
    let (handle, tx) = scene::spawn(ci, Duration::from_secs(1));
    tokio::spawn(crate::demo::run(tx, 42));
    Ok(vec![handle])
}

async fn live_scenes(store: &Store) -> anyhow::Result<Vec<SceneHandle>> {
    let stored = store.scene_settings("ci").await?;
    let settings: CiSettings = match stored.map(serde_json::from_value) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            tracing::warn!("stored CI settings are unreadable ({e}); using defaults");
            CiSettings::default()
        }
        None => CiSettings::default(),
    };
    let ci = CiScene::new(settings).or_else(|e| {
        tracing::warn!("stored CI settings are invalid ({e:#}); using defaults");
        CiScene::new(CiSettings::default())
    })?;
    // Sources arrive with the next plans; until then the scene only ticks.
    let (handle, _inputs) = scene::spawn(ci, Duration::from_secs(1));
    Ok(vec![handle])
}

/// Everything the live server needs, from an open store.
pub async fn live_state(store: Store, secrets: SecretBox, public_url: Option<&str>) -> anyhow::Result<AppState> {
    let scenes = live_scenes(&store).await?;
    let history = scenes
        .iter()
        .map(|h| {
            let status = Arc::new(Mutex::new(HistoryStatus::default()));
            history::spawn_writer(store.clone(), h.clone(), status.clone());
            (h.id, status)
        })
        .collect();
    history::spawn_pruner(store.clone());
    auth::announce_setup_code(&store, now_ms()).await?;
    let access = store.setting::<Access>(auth::ACCESS_KEY).await?.unwrap_or_default();
    let cookie_secure = public_url.is_some_and(|u| u.starts_with("https://"));
    if !cookie_secure {
        tracing::warn!("LORVAQ_PUBLIC_URL is not an https URL, so the session cookie goes without the Secure flag; set it in production");
    }
    let live = Live { store, secrets, cookie_secure, access: RwLock::new(access), limiter: Mutex::new(LoginLimiter::default()), history };
    Ok(AppState { scenes, live: Some(live) })
}

pub async fn run_server(args: ServerArgs) -> anyhow::Result<()> {
    if args.demo {
        return serve(args.listen, AppState::demo(demo_scenes()?)).await;
    }
    let url = args.db.database_url.context("DATABASE_URL is required (or run with --demo)")?;
    let key = args.secret_key.context("LORVAQ_SECRET_KEY is required: create one with `lorvaq keygen`")?;
    let secrets = SecretBox::from_base64(&key)?;
    let store = Store::connect(&url).await?;
    let state = live_state(store, secrets, args.public_url.as_deref()).await?;
    serve(args.listen, state).await
}

/// `lorvaq admin reset-password`: works while the server runs.
pub async fn reset_password(db: DbArgs) -> anyhow::Result<()> {
    let url = db.database_url.context("DATABASE_URL is required")?;
    let store = Store::connect(&url).await?;
    store.reset_admin().await?;
    let code = auth::issue_setup_code(&store, now_ms()).await?;
    println!("The admin password is removed and everybody is signed out.");
    println!("Open lorvaq and enter this one-time setup code (valid for 24 hours): {code}");
    Ok(())
}

pub async fn serve(listen: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let app = router(Arc::new(state)).into_make_service_with_connect_info::<SocketAddr>();
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "lorvaq is listening");
    let signal = shutdown_signal().shared();
    let server = axum::serve(listener, app).with_graceful_shutdown(signal.clone());
    let deadline = async {
        signal.await;
        tokio::time::sleep(STREAM_GRACE).await;
    };
    tokio::select! {
        result = server => result?,
        _ = deadline => tracing::info!("closed the remaining event streams"),
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutting down");
}
```

Заменить `src/main.rs` целиком:

```rust
use clap::Parser;
use lorvaq::app;
use lorvaq::cli::{AdminAction, Cli, Command};
use lorvaq::crypto::SecretBox;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))).init();
    match Cli::parse().command {
        Command::Keygen => {
            println!("{}", SecretBox::generate_key());
            Ok(())
        }
        Command::Server(args) => app::run_server(args).await,
        Command::Admin { action: AdminAction::ResetPassword, db } => app::reset_password(db).await,
    }
}
```

- [ ] **Step 3: Запустить — всё проходит, проверить вживую**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: все тесты зелёные, `live_smoke` — `2 passed`.

Затем руками (docker должен работать):

```bash
docker run -d --name lorvaq-pg -e POSTGRES_PASSWORD=dev -p 55432:5432 postgres:17-alpine
sleep 3
export DATABASE_URL=postgres://postgres:dev@127.0.0.1:55432/postgres
export LORVAQ_SECRET_KEY=$(cargo run -q -- keygen)
cargo run -q -- server --listen 127.0.0.1:8080 2>&1 | tee /tmp/lorvaq-live.log &
sleep 4
grep -o 'setup code [A-Z0-9-]*' /tmp/lorvaq-live.log
curl -s 127.0.0.1:8080/api/v1/me; echo
cargo run -q -- admin reset-password
kill %1; docker rm -f lorvaq-pg
```

Expected: в логе строка с кодом `XXXX-XXXX-XXXX-XXXX`; `me` — `{"admin":false,"demo":false,"setup_needed":true,"viewer":false}`; `reset-password` печатает новый код, сервер в это время работает.

- [ ] **Step 4: Commit**

```bash
git add src/ tests/
git commit -m "feat: live server with Postgres, history writer and admin reset-password

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 8: Веб — клиент API и проверки форм

**Files:**
- Replace: `web/src/api.ts`
- Modify: `web/src/api.test.ts` — дописать тест «locked»; `web/src/i18n.ts` — заменить целиком
- Create: `web/src/settings/forms.ts`, `web/src/settings/forms.test.ts`

**Interfaces:**
- Consumes: HTTP-контракт задачи 6.
- Produces:
  - `api.ts`: всё прежнее (`fetchScenes`, `fetchSnapshot`, `follow`, `Subscription`) плюс `AuthError`, `ApiError(status, message)`, типы `Me`, `Access`, `HistoryInfo`, `ExitRule`, `CiSettings`; функции `fetchMe`, `setup`, `login`, `logout`, `changePassword`, `getAccess`, `putAccess`, `getHistory`, `putHistory`, `getSceneSettings`, `putSceneSettings`. `follow(..., onStatus)` теперь сообщает и `"locked"` (401 на снимке) и после этого прекращает попытки.
  - `forms.ts`: `MIN_PASSWORD = 12`, `passwordProblem(pw, confirm)`, `parseList(text)`, `accessProblem(Access)`, `exitsProblem(ExitRule[])`, `parseCapacities(text) -> Record<string, number> | string`, `formatCapacities(record)`, `retentionProblem(days)` — каждая возвращает `null` или текст ошибки из `i18n`.

- [ ] **Step 1: Написать падающие тесты**

`web/src/settings/forms.test.ts`:

```ts
import { describe, expect, test } from "vitest";
import { accessProblem, exitsProblem, formatCapacities, parseCapacities, parseList, passwordProblem, retentionProblem } from "./forms";

describe("forms", () => {
  test("passwords", () => {
    expect(passwordProblem("short", "short")).toMatch(/12/);
    expect(passwordProblem("a long enough one", "a long enough two")).toMatch(/match/);
    expect(passwordProblem("a long enough one", "a long enough one")).toBeNull();
  });

  test("lists split on lines and commas", () => {
    expect(parseList(" 10.0.0.0/8,\n192.168.1.5 \n\n")).toEqual(["10.0.0.0/8", "192.168.1.5"]);
  });

  test("access", () => {
    expect(accessProblem({ mode: "open" })).toBeNull();
    expect(accessProblem({ mode: "proxy", header: "x-forwarded-user", trusted: ["10.0.0.0/8", "::1"] })).toBeNull();
    expect(accessProblem({ mode: "proxy", header: "bad header", trusted: ["10.0.0.1"] })).toMatch(/header/);
    expect(accessProblem({ mode: "proxy", header: "x-u", trusted: [] })).toMatch(/trusted/);
    expect(accessProblem({ mode: "proxy", header: "x-u", trusted: ["proxy.local"] })).toMatch(/proxy\.local/);
  });

  test("exits", () => {
    expect(exitsProblem([{ label: "prod", stage: "deploy" }])).toBeNull();
    expect(exitsProblem([{ label: "", stage: "deploy" }])).toMatch(/name/);
    expect(exitsProblem([{ label: "a", stage: "x" }, { label: "a", name: "y" }])).toMatch(/twice/);
    expect(exitsProblem([{ label: "a" }])).toMatch(/stage or a job name/);
    expect(exitsProblem([{ label: "a", stage: "(" }])).toMatch(/pattern/);
  });

  test("capacities", () => {
    expect(parseCapacities("shared = 5\ndeploy=1\n")).toEqual({ shared: 5, deploy: 1 });
    expect(parseCapacities("")).toEqual({});
    expect(parseCapacities("shared")).toMatch(/shared/);
    expect(parseCapacities("shared=0")).toMatch(/shared/);
    expect(formatCapacities({ deploy: 1, shared: 5 })).toBe("deploy=1\nshared=5");
  });

  test("retention", () => {
    expect(retentionProblem(14)).toBeNull();
    expect(retentionProblem(0)).not.toBeNull();
    expect(retentionProblem(366)).not.toBeNull();
    expect(retentionProblem(1.5)).not.toBeNull();
  });
});
```

Дописать в `web/src/api.test.ts`, внутрь `describe("follow", ...)`:

```ts
  test("a 401 on the snapshot reports locked and stops", async () => {
    let calls = 0;
    vi.stubGlobal("fetch", async () => {
      calls += 1;
      return { ok: false, status: 401, json: async () => ({ error: "sign in" }) };
    });
    vi.stubGlobal("EventSource", FakeEventSource);
    const statuses: string[] = [];
    follow("ci", () => {}, (s) => statuses.push(s));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(statuses).toEqual(["locked"]);
    expect(calls).toBe(1);
    expect(FakeEventSource.opened).toHaveLength(0);
  });
```

Run: `cd web && npm test`
Expected: FAIL — `Failed to resolve import "./forms"` и новый тест `follow` не получает `"locked"`.

- [ ] **Step 2: Реализовать**

Заменить `web/src/i18n.ts` целиком:

```ts
// Every UI string lives here.
export const t = {
  connecting: "connecting…",
  live: "live",
  reconnecting: "reconnecting…",
  offline: "server unreachable",
  locked: "signed out",
  stale: (source: string, seconds: number) => `${source} data is ${seconds}s old — showing the last known state`,
  free: "free",
  running: "running",
  disk: "disk",
  agentOffline: "agent offline",
  slots: (n: number) => `${n} slot${n === 1 ? "" : "s"}`,
  // account
  setupTitle: "Create the admin",
  setupHint: "lorvaq printed a one-time setup code to its log (or run `lorvaq admin reset-password`).",
  setupCode: "Setup code",
  password: "Password",
  confirmPassword: "Repeat the password",
  createAdmin: "Create admin",
  signInTitle: "Sign in",
  signIn: "Sign in",
  signOut: "Sign out",
  settings: "Settings",
  scenes: "Scenes",
  passwordTooShort: (n: number) => `Use at least ${n} characters`,
  passwordMismatch: "The passwords don't match",
  // settings
  save: "Save",
  saved: "Saved",
  accessTitle: "Who sees the scenes",
  accessOpen: "Anyone who can reach lorvaq",
  accessProxy: "People your reverse proxy signed in",
  accessAdmin: "Only the admin",
  proxyHeader: "Header with the user name",
  proxyTrusted: "Trusted proxy addresses or networks, one per line",
  badHeader: "Use letters, digits and dashes for the header name",
  noTrusted: "List at least one trusted proxy address",
  badNetwork: (value: string) => `${value} is not an address or a network`,
  ciTitle: "CI scene",
  exits: "Exits: a successful job matching a rule flies there",
  exitLabel: "Name",
  exitStage: "Stage pattern",
  exitName: "Job name pattern",
  addExit: "Add exit",
  remove: "Remove",
  maxProjects: "Projects with their own place",
  defaultCapacity: "Slots of a runner whose size is unknown",
  capacities: "Slots per runner process, one `name=slots` per line",
  exitNoLabel: "Every exit needs a name",
  exitTwice: (label: string) => `The exit ${label} is listed twice`,
  exitNoRule: (label: string) => `The exit ${label} needs a stage or a job name pattern`,
  badPattern: (pattern: string) => `${pattern} is not a valid pattern`,
  badCapacity: (line: string) => `"${line}" should look like name=slots with slots of 1 or more`,
  historyTitle: "History",
  retention: "Keep history for, days",
  badRetention: "Keep history from 1 to 365 days",
  historyQueue: (id: string, queued: number, dropped: number) => `${id}: ${queued} waiting to be written, ${dropped} dropped`,
  passwordTitle: "Admin password",
  currentPassword: "Current password",
  newPassword: "New password",
  changePassword: "Change password",
};
```

`web/src/settings/forms.ts`:

```ts
import type { Access, ExitRule } from "../api";
import { t } from "../i18n";

export const MIN_PASSWORD = 12;
const HEADER = /^[A-Za-z0-9-]+$/;
const NETWORK = /^[0-9A-Fa-f:.]+(\/\d{1,3})?$/;

export function passwordProblem(password: string, confirm: string): string | null {
  if ([...password].length < MIN_PASSWORD) return t.passwordTooShort(MIN_PASSWORD);
  if (password !== confirm) return t.passwordMismatch;
  return null;
}

export function parseList(text: string): string[] {
  return text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);
}

/** A first check in the browser; the server has the final word. */
export function accessProblem(access: Access): string | null {
  if (access.mode !== "proxy") return null;
  if (!HEADER.test(access.header)) return t.badHeader;
  if (access.trusted.length === 0) return t.noTrusted;
  const bad = access.trusted.find((x) => !NETWORK.test(x));
  return bad === undefined ? null : t.badNetwork(bad);
}

export function exitsProblem(exits: ExitRule[]): string | null {
  const seen = new Set<string>();
  for (const exit of exits) {
    const label = exit.label.trim();
    if (!label) return t.exitNoLabel;
    if (seen.has(label)) return t.exitTwice(label);
    seen.add(label);
    if (!exit.stage && !exit.name) return t.exitNoRule(label);
    for (const pattern of [exit.stage, exit.name]) {
      if (!pattern) continue;
      try {
        new RegExp(`^(?:${pattern})$`);
      } catch {
        return t.badPattern(pattern);
      }
    }
  }
  return null;
}

export function parseCapacities(text: string): Record<string, number> | string {
  const out: Record<string, number> = {};
  for (const line of text.split("\n").map((l) => l.trim()).filter(Boolean)) {
    const [name, slots, ...rest] = line.split("=").map((s) => s.trim());
    const n = Number(slots);
    if (!name || rest.length > 0 || !Number.isInteger(n) || n < 1) return t.badCapacity(line);
    out[name] = n;
  }
  return out;
}

export function formatCapacities(capacities: Record<string, number>): string {
  return Object.keys(capacities).sort().map((k) => `${k}=${capacities[k]}`).join("\n");
}

export function retentionProblem(days: number): string | null {
  return Number.isInteger(days) && days >= 1 && days <= 365 ? null : t.badRetention;
}
```

Заменить `web/src/api.ts` целиком:

```ts
import { ClientWorld, type Effect } from "./world";
import type { SceneInfo, Snapshot, WorldEvent } from "./types";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** 401: the viewer or admin has to sign in. */
export class AuthError extends Error {
  constructor() {
    super("sign in required");
  }
}

export class ApiError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
  }
}

async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 401) throw new AuthError();
  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const message = (data as { error?: string } | null)?.error ?? `${method} ${url}: ${res.status}`;
    throw new ApiError(res.status, message);
  }
  return data as T;
}

const getJson = <T>(url: string) => request<T>("GET", url);

export interface Me {
  demo: boolean;
  setup_needed: boolean;
  admin: boolean;
  viewer: boolean;
}

export type Access = { mode: "open" } | { mode: "admin" } | { mode: "proxy"; header: string; trusted: string[] };

export interface HistoryInfo {
  retention_days: number;
  scenes: { id: string; queued: number; dropped: number; written: number; last_error: string | null }[];
}

export interface ExitRule {
  label: string;
  stage?: string | null;
  name?: string | null;
}

/** The fields the settings page edits; the rest is sent back untouched. */
export interface CiSettings {
  exits: ExitRule[];
  max_projects: number;
  default_capacity: number;
  capacities: Record<string, number>;
  [key: string]: unknown;
}

export const fetchScenes = () => getJson<SceneInfo[]>("/api/v1/scenes");
export const fetchSnapshot = (id: string) => getJson<Snapshot>(`/api/v1/scenes/${encodeURIComponent(id)}/snapshot`);
export const fetchMe = () => getJson<Me>("/api/v1/me");
export const setup = (code: string, password: string) => request<{ ok: true }>("POST", "/api/v1/setup", { code, password });
export const login = (password: string) => request<{ ok: true }>("POST", "/api/v1/login", { password });
export const logout = () => request<{ ok: true }>("POST", "/api/v1/logout");
export const changePassword = (current: string, next: string) => request<{ ok: true }>("POST", "/api/v1/admin/password", { current, new: next });
export const getAccess = () => getJson<Access>("/api/v1/admin/access");
export const putAccess = (access: Access) => request<Access>("PUT", "/api/v1/admin/access", access);
export const getHistory = () => getJson<HistoryInfo>("/api/v1/admin/history");
export const putHistory = (retention_days: number) => request<{ retention_days: number }>("PUT", "/api/v1/admin/history", { retention_days });
export const getSceneSettings = <T>(id: string) => getJson<T>(`/api/v1/admin/scenes/${encodeURIComponent(id)}/settings`);
export const putSceneSettings = <T>(id: string, settings: T) => request<T>("PUT", `/api/v1/admin/scenes/${encodeURIComponent(id)}/settings`, settings);

export interface Subscription {
  close(): void;
}

export type FollowStatus = "live" | "reconnecting" | "locked";

/**
 * Keeps a world in sync: snapshot, then events after its seq. Any break —
 * network, server restart, 410 for a gap — means a fresh snapshot. A 401 means
 * the viewer must sign in: report "locked" and stop.
 */
export function follow(sceneId: string, onWorld: (world: ClientWorld, effects: Effect[]) => void, onStatus: (status: FollowStatus) => void): Subscription {
  let closed = false;
  let source: EventSource | null = null;
  let streamDone: (() => void) | null = null;

  const stream = (world: ClientWorld) =>
    new Promise<void>((done) => {
      if (closed) {
        done();
        return;
      }
      streamDone = done;
      source = new EventSource(`/api/v1/scenes/${encodeURIComponent(sceneId)}/events?since=${world.seq}`);
      source.onopen = () => onStatus("live");
      source.onmessage = (m: MessageEvent<string>) => onWorld(world, [world.apply(JSON.parse(m.data) as WorldEvent)]);
      source.onerror = () => {
        source?.close();
        done();
      };
    });

  void (async () => {
    while (!closed) {
      try {
        const snapshot = await fetchSnapshot(sceneId);
        if (closed) break;
        const world = ClientWorld.fromSnapshot(snapshot);
        onWorld(world, []);
        await stream(world);
      } catch (e) {
        if (e instanceof AuthError) {
          onStatus("locked");
          return;
        }
        // Server unreachable: retry below.
      }
      if (!closed) {
        onStatus("reconnecting");
        await sleep(1500);
      }
    }
  })();

  return {
    close() {
      closed = true;
      source?.close();
      streamDone?.();
    },
  };
}
```

В `web/src/scenes/ci/CiView.tsx` заменить тип `onStatus` в `Props`:

```ts
  onStatus: (status: "live" | "reconnecting" | "locked") => void;
```

- [ ] **Step 3: Запустить — тесты и сборка проходят**

Run: `cd web && npm test && npm run build`
Expected: `20 passed` (13 прежних + 6 форм + 1 «locked»), сборка без ошибок.

- [ ] **Step 4: Commit**

```bash
git add web/src/
git commit -m "feat(web): auth-aware API client and settings form checks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 9: Веб — страницы настройки, входа и настроек

**Files:**
- Create: `web/src/pages/Setup.tsx`, `web/src/pages/Login.tsx`, `web/src/pages/Settings.tsx`
- Replace: `web/src/App.tsx`
- Modify: `web/src/index.css` — дописать стили форм

**Interfaces:**
- Consumes: `api.ts`, `forms.ts`, `i18n.ts` задачи 8.
- Produces: маршрутизация по `location.hash`: `#/settings` — настройки (только администратору), всё прочее — сцены. Порядок экранов: нет ответа `me` → «connecting…»; `setup_needed` → «Создать администратора»; не `viewer` → «Вход»; иначе сцены. В шапке: вкладки сцен, «Settings» и «Sign out» для администратора, «Sign in» для зрителя в живом режиме.

Страницы — простая разметка без юнит-тестов (логика проверок уже покрыта в задаче 8); проверка — ручная, в задаче 10.

- [ ] **Step 1: Написать страницы**

`web/src/pages/Setup.tsx`:

```tsx
import { useState, type FormEvent } from "react";
import { ApiError, setup } from "../api";
import { t } from "../i18n";
import { passwordProblem } from "../settings/forms";

export function Setup({ onDone }: { onDone: () => void }) {
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = passwordProblem(password, confirm);
    if (problem) return setError(problem);
    setBusy(true);
    try {
      await setup(code, password);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="card narrow" onSubmit={submit}>
      <h2>{t.setupTitle}</h2>
      <p className="hint">{t.setupHint}</p>
      <label>
        {t.setupCode}
        <input value={code} onChange={(e) => setCode(e.target.value)} autoComplete="off" placeholder="ABCD-EFGH-JKMN-PQRS" required />
      </label>
      <label>
        {t.password}
        <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required />
      </label>
      <label>
        {t.confirmPassword}
        <input type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required />
      </label>
      {error && <p className="error">{error}</p>}
      <button type="submit" disabled={busy}>{t.createAdmin}</button>
    </form>
  );
}
```

`web/src/pages/Login.tsx`:

```tsx
import { useState, type FormEvent } from "react";
import { ApiError, AuthError, login } from "../api";
import { t } from "../i18n";

export function Login({ onDone }: { onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await login(password);
      onDone();
    } catch (err) {
      setError(err instanceof AuthError ? "Wrong password" : err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="card narrow" onSubmit={submit}>
      <h2>{t.signInTitle}</h2>
      <label>
        {t.password}
        <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" autoFocus required />
      </label>
      {error && <p className="error">{error}</p>}
      <button type="submit" disabled={busy}>{t.signIn}</button>
    </form>
  );
}
```

`web/src/pages/Settings.tsx`:

```tsx
import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import {
  ApiError,
  changePassword,
  getAccess,
  getHistory,
  getSceneSettings,
  putAccess,
  putHistory,
  putSceneSettings,
  type Access,
  type CiSettings,
  type ExitRule,
  type HistoryInfo,
} from "../api";
import { t } from "../i18n";
import { accessProblem, exitsProblem, formatCapacities, parseCapacities, parseList, passwordProblem, retentionProblem } from "../settings/forms";

const message = (err: unknown) => (err instanceof ApiError ? err.message : String(err));

function Section({ title, children, onSubmit, status }: { title: string; children: ReactNode; onSubmit: (e: FormEvent) => void; status: string | null }) {
  return (
    <form className="card" onSubmit={onSubmit}>
      <h2>{title}</h2>
      {children}
      <div className="row">
        <button type="submit">{t.save}</button>
        {status && <span className={status === t.saved ? "ok" : "error"}>{status}</span>}
      </div>
    </form>
  );
}

function AccessSection() {
  const [mode, setMode] = useState<Access["mode"]>("admin");
  const [header, setHeader] = useState("x-forwarded-user");
  const [trusted, setTrusted] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getAccess()
      .then((a) => {
        setMode(a.mode);
        if (a.mode === "proxy") {
          setHeader(a.header);
          setTrusted(a.trusted.join("\n"));
        }
      })
      .catch((e) => setStatus(message(e)));
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    const access: Access = mode === "proxy" ? { mode, header: header.trim(), trusted: parseList(trusted) } : { mode };
    const problem = accessProblem(access);
    if (problem) return setStatus(problem);
    try {
      await putAccess(access);
      setStatus(t.saved);
    } catch (err) {
      setStatus(message(err));
    }
  }

  return (
    <Section title={t.accessTitle} onSubmit={save} status={status}>
      {([["open", t.accessOpen], ["proxy", t.accessProxy], ["admin", t.accessAdmin]] as const).map(([value, label]) => (
        <label key={value} className="choice">
          <input type="radio" name="mode" checked={mode === value} onChange={() => setMode(value)} />
          {label}
        </label>
      ))}
      {mode === "proxy" && (
        <>
          <label>
            {t.proxyHeader}
            <input value={header} onChange={(e) => setHeader(e.target.value)} />
          </label>
          <label>
            {t.proxyTrusted}
            <textarea rows={3} value={trusted} onChange={(e) => setTrusted(e.target.value)} placeholder="10.0.0.0/8" />
          </label>
        </>
      )}
    </Section>
  );
}

function CiSection() {
  const [settings, setSettings] = useState<CiSettings | null>(null);
  const [exits, setExits] = useState<ExitRule[]>([]);
  const [capacities, setCapacities] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getSceneSettings<CiSettings>("ci")
      .then((s) => {
        setSettings(s);
        setExits(s.exits);
        setCapacities(formatCapacities(s.capacities));
      })
      .catch((e) => setStatus(message(e)));
  }, []);

  if (!settings) return status ? <p className="error">{status}</p> : null;

  const setExit = (i: number, patch: Partial<ExitRule>) => setExits(exits.map((x, j) => (j === i ? { ...x, ...patch } : x)));

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!settings) return;
    const cleaned = exits.map((x) => ({ label: x.label.trim(), stage: x.stage?.trim() || null, name: x.name?.trim() || null }));
    const parsed = parseCapacities(capacities);
    const problem = exitsProblem(cleaned) ?? (typeof parsed === "string" ? parsed : null);
    if (problem || typeof parsed === "string") return setStatus(problem);
    try {
      const applied = await putSceneSettings<CiSettings>("ci", { ...settings, exits: cleaned, capacities: parsed });
      setSettings(applied);
      setStatus(t.saved);
    } catch (err) {
      setStatus(message(err));
    }
  }

  return (
    <Section title={t.ciTitle} onSubmit={save} status={status}>
      <p className="hint">{t.exits}</p>
      {exits.map((x, i) => (
        <div className="exit" key={i}>
          <input aria-label={t.exitLabel} placeholder={t.exitLabel} value={x.label} onChange={(e) => setExit(i, { label: e.target.value })} />
          <input aria-label={t.exitStage} placeholder={t.exitStage} value={x.stage ?? ""} onChange={(e) => setExit(i, { stage: e.target.value })} />
          <input aria-label={t.exitName} placeholder={t.exitName} value={x.name ?? ""} onChange={(e) => setExit(i, { name: e.target.value })} />
          <button type="button" onClick={() => setExits(exits.filter((_, j) => j !== i))}>{t.remove}</button>
        </div>
      ))}
      <button type="button" onClick={() => setExits([...exits, { label: "", stage: "", name: "" }])}>{t.addExit}</button>
      <label>
        {t.maxProjects}
        <input type="number" min={1} value={settings.max_projects} onChange={(e) => setSettings({ ...settings, max_projects: Number(e.target.value) })} />
      </label>
      <label>
        {t.defaultCapacity}
        <input type="number" min={1} value={settings.default_capacity} onChange={(e) => setSettings({ ...settings, default_capacity: Number(e.target.value) })} />
      </label>
      <label>
        {t.capacities}
        <textarea rows={3} value={capacities} onChange={(e) => setCapacities(e.target.value)} placeholder="shared=5" />
      </label>
    </Section>
  );
}

function HistorySection() {
  const [info, setInfo] = useState<HistoryInfo | null>(null);
  const [days, setDays] = useState(14);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getHistory()
      .then((h) => {
        setInfo(h);
        setDays(h.retention_days);
      })
      .catch((e) => setStatus(message(e)));
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    const problem = retentionProblem(days);
    if (problem) return setStatus(problem);
    try {
      await putHistory(days);
      setStatus(t.saved);
    } catch (err) {
      setStatus(message(err));
    }
  }

  return (
    <Section title={t.historyTitle} onSubmit={save} status={status}>
      <label>
        {t.retention}
        <input type="number" min={1} max={365} value={days} onChange={(e) => setDays(Number(e.target.value))} />
      </label>
      {info?.scenes.map((s) => (
        <p key={s.id} className={s.last_error ? "error" : "hint"}>
          {t.historyQueue(s.id, s.queued, s.dropped)}
          {s.last_error ? ` — ${s.last_error}` : ""}
        </p>
      ))}
    </Section>
  );
}

function PasswordSection() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  async function save(e: FormEvent) {
    e.preventDefault();
    const problem = passwordProblem(next, confirm);
    if (problem) return setStatus(problem);
    try {
      await changePassword(current, next);
      setCurrent("");
      setNext("");
      setConfirm("");
      setStatus(t.saved);
    } catch (err) {
      setStatus(message(err));
    }
  }

  return (
    <Section title={t.passwordTitle} onSubmit={save} status={status}>
      <label>
        {t.currentPassword}
        <input type="password" value={current} onChange={(e) => setCurrent(e.target.value)} autoComplete="current-password" required />
      </label>
      <label>
        {t.newPassword}
        <input type="password" value={next} onChange={(e) => setNext(e.target.value)} autoComplete="new-password" required />
      </label>
      <label>
        {t.confirmPassword}
        <input type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required />
      </label>
    </Section>
  );
}

export function Settings() {
  return (
    <div className="settings">
      <AccessSection />
      <CiSection />
      <HistorySection />
      <PasswordSection />
    </div>
  );
}
```

Заменить `web/src/App.tsx` целиком:

```tsx
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { fetchMe, fetchScenes, logout, type FollowStatus, type Me } from "./api";
import { t } from "./i18n";
import type { SceneInfo } from "./types";
import { CiView } from "./scenes/ci/CiView";
import { Login } from "./pages/Login";
import { Settings } from "./pages/Settings";
import { Setup } from "./pages/Setup";

type Status = "connecting" | "offline" | FollowStatus;
type Route = "scenes" | "settings";

const routeOf = (hash: string): Route => (hash === "#/settings" ? "settings" : "scenes");

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [scenes, setScenes] = useState<SceneInfo[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>("connecting");
  const [stale, setStale] = useState<Record<string, number>>({});
  const [now, setNow] = useState(Date.now());
  const [route, setRoute] = useState<Route>(routeOf(location.hash));

  const refresh = useCallback(() => {
    fetchMe()
      .then((m) => {
        setMe(m);
        if (m.viewer) {
          fetchScenes()
            .then((list) => {
              setScenes(list);
              setActive((current) => current ?? list[0]?.id ?? null);
            })
            .catch(() => setStatus("offline"));
        }
      })
      .catch(() => setStatus("offline"));
  }, []);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const onHash = () => setRoute(routeOf(location.hash));
    window.addEventListener("hashchange", onHash);
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      window.removeEventListener("hashchange", onHash);
      clearInterval(id);
    };
  }, []);

  const onStatus = useCallback(
    (s: FollowStatus) => {
      setStatus(s);
      if (s === "locked") refresh();
    },
    [refresh],
  );

  const signOut = () => {
    void logout().finally(() => {
      location.hash = "";
      refresh();
    });
  };

  let main: ReactNode = null;
  if (!me) main = <p className="hint pad">{t[status]}</p>;
  else if (me.setup_needed) main = <Setup onDone={refresh} />;
  else if (route === "settings" && !me.admin) main = <Login onDone={refresh} />;
  else if (route === "settings") main = <Settings />;
  else if (!me.viewer) main = <Login onDone={refresh} />;
  else if (active === "ci") main = <CiView onStatus={onStatus} onStale={setStale} />;

  return (
    <div className="app">
      <header>
        <h1>lorvaq</h1>
        <nav className="tabs">
          {scenes.map((s) => (
            <button
              key={s.id}
              className={route === "scenes" && s.id === active ? "on" : ""}
              onClick={() => {
                setActive(s.id);
                location.hash = "";
              }}
            >
              {s.title}
            </button>
          ))}
        </nav>
        <span className="status">{t[status]}</span>
        {me && !me.demo && me.admin && (
          <>
            <a href="#/settings">{t.settings}</a>
            <button className="link" onClick={signOut}>{t.signOut}</button>
          </>
        )}
        {me && !me.demo && !me.admin && !me.setup_needed && <a href="#/settings">{t.signIn}</a>}
      </header>
      {route === "scenes" &&
        Object.entries(stale).map(([source, since]) => (
          <div key={source} className="banner">
            {t.stale(source, Math.max(0, Math.round((now - since) / 1000)))}
          </div>
        ))}
      {main}
    </div>
  );
}
```

Дописать в конец `web/src/index.css`:

```css
.pad { padding: 20px; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 16px 20px; margin: 16px 20px; display: flex; flex-direction: column; gap: 10px; max-width: 720px; }
.card.narrow { max-width: 380px; margin: 48px auto; }
.card h2 { font-size: 15px; margin: 0; }
.card label { display: flex; flex-direction: column; gap: 4px; font-size: 13px; color: var(--muted); }
.card label.choice { flex-direction: row; align-items: center; gap: 8px; color: var(--text); }
.card input, .card textarea { font: inherit; color: var(--text); background: var(--bg); border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px; }
.card button, header button.link { font: inherit; color: var(--text); background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px; padding: 6px 12px; cursor: pointer; align-self: flex-start; }
header a, header button.link { color: var(--muted); font-size: 13px; text-decoration: none; background: none; border: none; padding: 0; }
.hint { color: var(--muted); font-size: 13px; margin: 0; }
.error { color: var(--bad); font-size: 13px; margin: 0; }
.ok { color: var(--ok); font-size: 13px; }
.row { display: flex; align-items: center; gap: 12px; }
.exit { display: grid; grid-template-columns: 1fr 1fr 1fr auto; gap: 8px; }
```

- [ ] **Step 2: Тесты и сборка**

Run: `cd web && npm test && npm run build`
Expected: `20 passed`, `tsc` и `vite build` без ошибок.

- [ ] **Step 3: Commit**

```bash
git add web/src/
git commit -m "feat(web): setup, sign-in and settings pages

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 10: Compose с Postgres, образ, README и проверка целиком

**Files:**
- Modify: `Dockerfile` — копировать `migrations/`
- Create: `compose.yml`
- Modify: `README.md`

**Interfaces:**
- Consumes: всё. `compose.yml` поднимает `postgres:17-alpine` и lorvaq в живом режиме; секреты — из `.env` рядом (не в репо).

- [ ] **Step 1: Файлы**

В `Dockerfile`, в стадии `build`, после строки `COPY src ./src` добавить:

```dockerfile
COPY migrations ./migrations
```

`compose.yml`:

```yaml
# lorvaq with its database. Put two values into .env next to this file:
#   POSTGRES_PASSWORD=<any long password>
#   LORVAQ_SECRET_KEY=<output of: openssl rand -base64 32>
# Then: docker compose up -d --build, and read the one-time setup code with:
#   docker compose logs lorvaq | grep "setup code"
services:
  db:
    image: postgres:17-alpine
    environment:
      POSTGRES_USER: lorvaq
      POSTGRES_DB: lorvaq
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:?set POSTGRES_PASSWORD in .env}
    volumes:
      - lorvaq-db:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U lorvaq -d lorvaq"]
      interval: 2s
      timeout: 3s
      retries: 30

  lorvaq:
    build: .
    image: lorvaq:dev
    command: ["server", "--listen", "0.0.0.0:8080"]
    environment:
      DATABASE_URL: postgres://lorvaq:${POSTGRES_PASSWORD}@db:5432/lorvaq
      LORVAQ_SECRET_KEY: ${LORVAQ_SECRET_KEY:?set LORVAQ_SECRET_KEY in .env (lorvaq keygen)}
      LORVAQ_PUBLIC_URL: ${LORVAQ_PUBLIC_URL:-}
    ports:
      - "8080:8080"
    depends_on:
      db:
        condition: service_healthy

volumes:
  lorvaq-db:
```

Дописать в `.gitignore` строку:

```text
.env
```

В `README.md` заменить раздел `## Try it` целиком:

```markdown
## Try it

On simulated data, no database:

    docker compose -f compose.demo.yml up --build

Open http://localhost:8080.

## Run it

lorvaq keeps its history and settings in Postgres. Create `.env` next to
`compose.yml`:

    POSTGRES_PASSWORD=<any long password>
    LORVAQ_SECRET_KEY=<output of: openssl rand -base64 32, or lorvaq keygen>
    LORVAQ_PUBLIC_URL=https://lorvaq.example.com   # optional; https turns on Secure cookies

Then:

    docker compose up -d --build
    docker compose logs lorvaq | grep "setup code"

Open lorvaq, enter the one-time setup code and choose the admin password. In
Settings decide who sees the scenes: anyone, people your reverse proxy signed
in, or only the admin. Lost the password? `docker compose exec lorvaq /lorvaq
admin reset-password` prints a new setup code and signs everybody out.
```

- [ ] **Step 2: Проверить целиком**

```bash
printf 'POSTGRES_PASSWORD=%s\n' "$(openssl rand -hex 16)" > .env
printf 'LORVAQ_SECRET_KEY=%s\n' "$(cargo run -q -- keygen)" >> .env
docker compose up -d --build
for i in $(seq 1 60); do curl -fsS localhost:8080/healthz >/dev/null 2>&1 && break; sleep 1; done
curl -fsS localhost:8080/healthz; echo
docker compose logs lorvaq | grep -o 'setup code [A-Z0-9-]*'
```

Expected: `healthz` — `"db":true`; в логе код настройки.

Открыть `http://localhost:8080` и проверить по шагам, записав в отчёт, что увидено:
1. Экран «Create the admin»: неверный код — ошибка; короткий пароль — ошибка в браузере; верный код и пароль — сразу вход, видна сцена CI (пустая: источников ещё нет — это ожидаемо).
2. В шапке «Settings» и «Sign out». «Settings»: режим доступа «Only the admin»; добавить выход `prod` со стадией `deploy` → «Saved»; выход с шаблоном `(` → ошибка, сохранение не проходит; срок истории `30` → «Saved»; в блоке истории — очередь `0`, ошибок нет.
3. «Sign out» → экран входа; неверный пароль 5 раз → на шестой «too many attempts».
4. `docker compose exec lorvaq /lorvaq admin reset-password` → новый код; обновить страницу → снова «Create the admin».
5. `docker compose restart lorvaq` → после перезапуска выход `prod` и режим доступа на месте.
6. `docker compose stop db` → схема продолжает открываться (режим `open` поставить заранее в настройках), в «Settings» история показывает растущую очередь и ошибку; `docker compose start db` → очередь уходит в 0.

```bash
docker compose down -v
rm .env
```

- [ ] **Step 3: Финальная проверка репо**

```bash
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
(cd web && npm test && npm run build)
git grep -n -i -E -f ~/.config/lorvaq/denylist -- ':!web/src/scenes/ci/renderer.ts' || echo "clean"
git status --short
```

Expected: всё зелёное; `clean`; `.env` не попадает в `git status`.

- [ ] **Step 4: Commit**

```bash
git add Dockerfile compose.yml README.md .gitignore
git commit -m "feat: compose with Postgres and how to run lorvaq for real

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
