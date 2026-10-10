# lorvaq — план 1: живая сцена CI на симуляции

> **Для агентов-исполнителей:** ОБЯЗАТЕЛЬНЫЙ ПОД-СКИЛЛ: используйте superpowers:subagent-driven-development (рекомендуется) или superpowers:executing-plans, чтобы выполнять план задача за задачей. Шаги размечены чекбоксами (`- [ ]`).

**Цель:** `docker compose -f compose.demo.yml up` поднимает lorvaq, и в браузере на `http://localhost:8080` живёт анимированная сцена CI на симулированных данных — через настоящее ядро (модель мира, редьюсер сцены, шина событий, снимок + SSE).

**Архитектура:** один Rust-крейт: модель мира (`world`), шина событий с буфером (`world::bus`), трейт сцены и её runtime (`scene`), сцена CI (`scenes::ci`: входы, подбор раннера, редьюсер), симулятор (`demo`), HTTP (`http`: API, SSE, вшитая статика). Фронт — React-оболочка и императивный SVG-рендерер сцены поверх чистой функции раскладки; собирается Vite и вшивается в бинарник через `rust-embed`.

**Стек:** Rust 1.91, axum 0.8, tokio 1, serde/serde_json, clap 4, tokio-stream 0.1, futures 0.3, rust-embed 8, regex 1, rand 0.9; React 19.3, Vite 8.3, Vitest 5, TypeScript 7.0; Docker (node:22-alpine → rust:1.91-alpine → scratch).

**Spec:** `docs/design/2026-10-10-core-and-ci-scene.md` — план реализует его разделы 1–5, 7, 9 (в части, не требующей базы и внешних источников), 10–12.

### Место плана в общей дорожке

Spec покрывает несколько подсистем, поэтому реализация разбита на пять планов; каждый даёт работающий результат:

| План | Результат |
|---|---|
| **1 (этот)** | ядро + сцена CI + симулятор + интерфейс + образ: живая сцена в `--demo` |
| 2 | Postgres (история, настройки, секреты), администратор и вход, режимы зрителей, страницы настроек |
| 3 | источник GitLab: мастер подключения (OAuth + PKCE, токен по ссылке), опрос GraphQL, «данные устарели» |
| 4 | разведка на живом раннере (Л1), `lorvaq agent --join`, join-токены, «Добавить хост» |
| 5 | GitHub Actions, образ в ghcr по тегу, `compose.yml`, README с демо, gitleaks |

## Глобальные ограничения

- Rust **1.91** (`rust-toolchain.toml`), edition 2021, `cargo clippy --all-targets -- -D warnings` и `cargo fmt --check` чистые после каждой задачи.
- Node ≥ 22; версии фронта: react/react-dom `^19.3.0`, vite `^8.3.4`, vitest `^5.0.3`, typescript `^7.0.2`, @vitejs/plugin-react `^6.1.2`, @types/react и @types/react-dom `^19.3.0`.
- В этом плане **нет базы**: история — только буфер последних **10 000** событий в памяти (spec §7).
- Контракт событий один для бэка и фронта: `{"seq":N,"ts_ms":N,"change":{"op":"<snake_case>",...}}`; места/сущности/уровни — как в spec §2. Любое изменение формата — в обеих сторонах в одной задаче.
- Время — миллисекунды Unix (`i64` в Rust, `number` в TS).
- **Ни следа конкретной студии** (spec §12): в симуляторе только проекты `demo/*`, хост `runner-host`, агенты `agent-*`; никаких реальных хостов, доменов, IP, скриншотов с реальными данными.
- Интерфейс — на английском (публичный продукт); все строки UI — в `web/src/i18n.ts`. Подписи зон приходят с сервера и тоже английские.
- `unwrap()` — только в тестах; в рабочем коде для заведомо невозможных ошибок — `expect("<почему невозможно>")`.
- Коммиты на английском, последней строкой: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Фокус ревью

Входы и сбои, которые spec подразумевает, но легко пропустить; у каждого есть тест в задаче-владельце:

1. **Агент быстрее GitLab:** в отчёте агента джоб, о котором GitLab ещё не сообщил, — фантомного джоба нет; когда GitLab сообщит, джоб сразу встаёт в свой слот (задача 6).
2. **Джоб перепрыгнул статусы между опросами:** из `pending` сразу в завершённые или впервые увиден уже `running` — без ошибок, с правильным местом и анимацией из проекта (задача 6).
3. **Номер слота больше объявленной вместимости** (`concurrent` меньше реального) — вместимость расширяется, слот существует, джоб в нём (задача 6).
4. **Клиент с устаревшим или «будущим» `Last-Event-ID`** (переполнение буфера, рестарт сервера) — `410`, клиент берёт снимок заново (задачи 3 и 8).
5. **Огромная очередь** (200 джобов) — 40 токенов и «+160», ничего не вылезает за зону (задача 11).

## Структура файлов

```text
lorvaq/
  Cargo.toml  Cargo.lock  rust-toolchain.toml  LICENSE  README.md
  .gitignore  .dockerignore  Dockerfile  compose.demo.yml
  src/
    main.rs            точка входа: CLI → app
    lib.rs             объявления модулей
    cli.rs             разбор аргументов (clap)
    app.rs             сборка сервера: сцены, симулятор, HTTP, остановка
    world/mod.rs       Place, Entity, Level, Change, Event, Snapshot, World
    world/bus.rs       EventBus: seq, буфер, подписка, since() / Gap
    scene.rs           трейт Scene, SceneHandle, runtime spawn()
    scenes/mod.rs
    scenes/ci/mod.rs
    scenes/ci/input.rs     входы сцены CI (GitLab, агент)
    scenes/ci/settings.rs  настройки и выходы
    scenes/ci/matching.rs  может ли раннер взять джоб
    scenes/ci/reduce.rs    CiScene: редьюсер и tick
    scenes/ci/testkit.rs   тестовая обвязка (только cfg(test))
    demo/mod.rs        симулятор и его цикл
    http/mod.rs        API, SSE
    http/static_files.rs   вшитый интерфейс
  tests/demo_smoke.rs
  web/
    package.json  package-lock.json  tsconfig.json  vite.config.ts  index.html
    src/main.tsx  App.tsx  index.css  i18n.ts
    src/types.ts  world.ts  world.test.ts  api.ts  colors.ts  colors.test.ts
    src/scenes/ci/layout.ts  layout.test.ts  renderer.ts  CiView.tsx
```

---

### Задача 1: Скелет крейта и CLI

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `LICENSE`, `src/lib.rs`, `src/cli.rs`, `src/main.rs`

**Interfaces:**
- Produces: `lorvaq::cli::{Cli, Command}`; `Command::Server { demo: bool, listen: SocketAddr }`.

- [ ] **Step 1: Создать манифесты**

`Cargo.toml`:

```toml
[package]
name = "lorvaq"
version = "0.1.0"
edition = "2021"
description = "Live scenes for real-time monitoring: see things move through your system"
license = "MIT"
repository = "https://github.com/alfinkly/lorvaq"

[dependencies]
anyhow = "1"
axum = "0.8"
clap = { version = "4", features = ["derive", "env"] }
futures = "0.3"
mime_guess = "2"
rand = { version = "0.9", features = ["small_rng"] }
regex = "1"
rust-embed = "8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-stream = { version = "0.1", features = ["sync"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
http-body-util = "0.1"
tower = { version = "0.5", features = ["util"] }

[profile.release]
strip = true
lto = true
opt-level = "z"
```

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.91"
components = ["clippy", "rustfmt"]
```

`.gitignore`:

```text
/target
/web/node_modules
/web/dist
```

`LICENSE`:

```text
MIT License

Copyright (c) 2026 Alimzhan Kangerey

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

`src/lib.rs`:

```rust
pub mod cli;
```

- [ ] **Step 2: Написать падающий тест CLI**

`src/cli.rs` (только тесты; типов ещё нет):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn server_demo_with_listen() {
        let cli = Cli::try_parse_from(["lorvaq", "server", "--demo", "--listen", "127.0.0.1:9000"]).unwrap();
        let Command::Server { demo, listen } = cli.command;
        assert!(demo);
        assert_eq!(listen.port(), 9000);
    }

    #[test]
    fn server_defaults_to_port_8080_without_demo() {
        let cli = Cli::try_parse_from(["lorvaq", "server"]).unwrap();
        let Command::Server { demo, listen } = cli.command;
        assert!(!demo);
        assert_eq!(listen.port(), 8080);
    }

    #[test]
    fn unknown_command_is_an_error() {
        assert!(Cli::try_parse_from(["lorvaq", "nope"]).is_err());
    }
}
```

- [ ] **Step 3: Запустить — тест падает**

Run: `cargo test cli::`
Expected: ошибка компиляции `cannot find type 'Cli' in this scope`.

- [ ] **Step 4: Реализовать CLI и main**

Добавить в начало `src/cli.rs`:

```rust
//! Command line: `lorvaq server [--demo] [--listen ADDR]`.

use clap::{Parser, Subcommand};
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
    Server {
        /// Use the built-in simulator instead of real sources (no database needed).
        #[arg(long)]
        demo: bool,
        /// Address to listen on.
        #[arg(long, env = "LORVAQ_LISTEN", default_value = "0.0.0.0:8080")]
        listen: SocketAddr,
    },
}
```

`src/main.rs` (временный — задача 9 заменит его целиком):

```rust
use clap::Parser;
use lorvaq::cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let Command::Server { demo, listen } = Cli::parse().command;
    anyhow::ensure!(demo, "this build only runs with --demo");
    tracing::info!(%listen, "nothing to serve yet");
    Ok(())
}
```

- [ ] **Step 5: Запустить — тесты проходят, линтеры чистые**

Run: `cargo test cli:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `3 passed`, без предупреждений. Если `fmt --check` ругается — `cargo fmt` и повторить.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore LICENSE src/
git commit -m "feat: crate skeleton and CLI

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 2: Модель мира

**Files:**
- Create: `src/world/mod.rs`
- Modify: `src/lib.rs` — добавить `pub mod world;`

**Interfaces:**
- Produces:
  - `type Id = String; type Attrs = BTreeMap<String, serde_json::Value>;`
  - `struct Place { id, kind: String, parent: Option<Id>, label: String, capacity: Option<u32>, attrs: Attrs }`
  - `struct Entity { id, kind: String, place: Id, state: String, attrs: Attrs, since_ms: i64 }`
  - `struct Level { owner: Id, metric: String, value: f64, max: Option<f64> }`
  - `enum Change` (serde `tag = "op"`, snake_case): `UpsertPlace { place }`, `RemovePlace { id }`, `Appear { entity }`, `Move { id, to }`, `SetState { id, state }`, `SetLevel { level }`, `ClearLevel { owner, metric }`, `Flash { at, kind }`, `Gone { id }`, `Stale { source, since_ms }`, `Fresh { source }`
  - `struct Event { seq: u64, ts_ms: i64, change: Change }`
  - `struct Snapshot { seq: u64, places: Vec<Place>, entities: Vec<Entity>, levels: Vec<Level>, stale: BTreeMap<String, i64> }`
  - `World`: `apply(&mut self, &Change, now_ms: i64) -> bool`, `place(&str)`, `entity(&str)`, `places()`, `entities()`, `level(owner, metric)`, `snapshot(seq) -> Snapshot`, `from_snapshot(&Snapshot) -> World`

- [ ] **Step 1: Написать падающие тесты**

`src/world/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn zone(id: &str) -> Place {
        Place { id: id.into(), kind: "zone".into(), parent: None, label: id.into(), capacity: None, attrs: Attrs::new() }
    }
    fn child(id: &str, parent: &str) -> Place {
        Place { parent: Some(parent.into()), ..zone(id) }
    }
    fn job(id: &str, place: &str) -> Entity {
        Entity { id: id.into(), kind: "job".into(), place: place.into(), state: "pending".into(), attrs: Attrs::new(), since_ms: 0 }
    }
    fn up(p: Place) -> Change {
        Change::UpsertPlace { place: p }
    }
    fn level(owner: &str, value: f64) -> Level {
        Level { owner: owner.into(), metric: "mem".into(), value, max: Some(10.0) }
    }

    #[test]
    fn appear_needs_an_existing_place_and_a_new_id() {
        let mut w = World::default();
        assert!(!w.apply(&Change::Appear { entity: job("j1", "queue") }, 1));
        assert!(w.apply(&up(zone("queue")), 1));
        assert!(w.apply(&Change::Appear { entity: job("j1", "queue") }, 1));
        assert!(!w.apply(&Change::Appear { entity: job("j1", "queue") }, 1), "same id twice");
    }

    #[test]
    fn move_changes_place_and_time_only_once() {
        let mut w = World::default();
        w.apply(&up(zone("queue")), 0);
        w.apply(&up(zone("slot")), 0);
        w.apply(&Change::Appear { entity: job("j1", "queue") }, 0);
        assert!(w.apply(&Change::Move { id: "j1".into(), to: "slot".into() }, 5));
        assert_eq!(w.entity("j1").unwrap().place, "slot");
        assert_eq!(w.entity("j1").unwrap().since_ms, 5);
        assert!(!w.apply(&Change::Move { id: "j1".into(), to: "slot".into() }, 6), "already there");
        assert!(!w.apply(&Change::Move { id: "j1".into(), to: "nowhere".into() }, 6), "missing place");
        assert!(!w.apply(&Change::Move { id: "ghost".into(), to: "queue".into() }, 6), "missing entity");
    }

    #[test]
    fn upsert_place_reports_only_real_changes() {
        let mut w = World::default();
        assert!(w.apply(&up(zone("q")), 0));
        assert!(!w.apply(&up(zone("q")), 0));
        let renamed = Place { label: "queue".into(), ..zone("q") };
        assert!(w.apply(&up(renamed), 0));
        assert_eq!(w.place("q").unwrap().label, "queue");
    }

    #[test]
    fn occupied_or_parent_places_are_not_removed() {
        let mut w = World::default();
        w.apply(&up(zone("host")), 0);
        w.apply(&up(child("slot", "host")), 0);
        w.apply(&Change::Appear { entity: job("j1", "slot") }, 0);
        assert!(!w.apply(&Change::RemovePlace { id: "host".into() }, 0), "has a child");
        assert!(!w.apply(&Change::RemovePlace { id: "slot".into() }, 0), "has an entity");
        w.apply(&Change::Gone { id: "j1".into() }, 0);
        assert!(w.apply(&Change::RemovePlace { id: "slot".into() }, 0));
        assert!(w.apply(&Change::RemovePlace { id: "host".into() }, 0));
        assert!(!w.apply(&Change::RemovePlace { id: "host".into() }, 0), "already gone");
    }

    #[test]
    fn levels_need_an_owner_and_leave_with_it() {
        let mut w = World::default();
        assert!(!w.apply(&Change::SetLevel { level: level("j1", 1.0) }, 0), "no owner yet");
        w.apply(&up(zone("q")), 0);
        w.apply(&Change::Appear { entity: job("j1", "q") }, 0);
        assert!(w.apply(&Change::SetLevel { level: level("j1", 1.0) }, 0));
        assert!(!w.apply(&Change::SetLevel { level: level("j1", 1.0) }, 0), "same value");
        assert!(w.apply(&Change::SetLevel { level: level("j1", 2.0) }, 0));
        assert_eq!(w.level("j1", "mem").unwrap().value, 2.0);
        w.apply(&Change::Gone { id: "j1".into() }, 0);
        assert!(w.level("j1", "mem").is_none());
    }

    #[test]
    fn clear_level_and_set_state() {
        let mut w = World::default();
        w.apply(&up(zone("q")), 0);
        w.apply(&Change::Appear { entity: job("j1", "q") }, 0);
        assert!(!w.apply(&Change::SetState { id: "j1".into(), state: "pending".into() }, 0), "same state");
        assert!(w.apply(&Change::SetState { id: "j1".into(), state: "running".into() }, 0));
        w.apply(&Change::SetLevel { level: level("j1", 1.0) }, 0);
        assert!(w.apply(&Change::ClearLevel { owner: "j1".into(), metric: "mem".into() }, 0));
        assert!(!w.apply(&Change::ClearLevel { owner: "j1".into(), metric: "mem".into() }, 0));
    }

    #[test]
    fn flash_needs_a_place_and_changes_nothing() {
        let mut w = World::default();
        assert!(!w.apply(&Change::Flash { at: "slot".into(), kind: "oom".into() }, 0));
        w.apply(&up(zone("slot")), 0);
        let before = w.clone();
        assert!(w.apply(&Change::Flash { at: "slot".into(), kind: "oom".into() }, 0));
        assert_eq!(w, before);
    }

    #[test]
    fn stale_and_fresh() {
        let mut w = World::default();
        assert!(w.apply(&Change::Stale { source: "gitlab".into(), since_ms: 10 }, 0));
        assert!(!w.apply(&Change::Stale { source: "gitlab".into(), since_ms: 10 }, 0));
        assert_eq!(w.snapshot(0).stale.get("gitlab"), Some(&10));
        assert!(w.apply(&Change::Fresh { source: "gitlab".into() }, 0));
        assert!(!w.apply(&Change::Fresh { source: "gitlab".into() }, 0));
    }

    #[test]
    fn snapshot_round_trips() {
        let mut w = World::default();
        w.apply(&up(zone("q")), 0);
        w.apply(&Change::Appear { entity: job("j1", "q") }, 0);
        w.apply(&Change::SetLevel { level: level("q", 3.0) }, 0);
        w.apply(&Change::Stale { source: "gitlab".into(), since_ms: 1 }, 0);
        let snap = w.snapshot(7);
        assert_eq!(snap.seq, 7);
        assert_eq!(World::from_snapshot(&snap), w);
    }

    #[test]
    fn wire_format_is_stable() {
        let mv = Change::Move { id: "j".into(), to: "q".into() };
        assert_eq!(serde_json::to_value(&mv).unwrap(), json!({"op": "move", "id": "j", "to": "q"}));
        let ev = Event { seq: 1, ts_ms: 2, change: Change::Gone { id: "j".into() } };
        assert_eq!(serde_json::to_value(&ev).unwrap(), json!({"seq": 1, "ts_ms": 2, "change": {"op": "gone", "id": "j"}}));
        let p = serde_json::to_value(up(zone("q"))).unwrap();
        assert_eq!(p["op"], "upsert_place");
        assert_eq!(p["place"]["parent"], serde_json::Value::Null);
    }
}
```

- [ ] **Step 2: Запустить — тесты падают**

Run: `cargo test world::`
Expected: ошибки компиляции `cannot find type 'Place'` и т.п.

- [ ] **Step 3: Реализовать модель**

Добавить в начало `src/world/mod.rs`:

```rust
//! The world every scene is described in: places, entities in places, levels
//! against limits, and the changes between them. Knows nothing about sources.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Id = String;
pub type Attrs = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub id: Id,
    pub kind: String,
    pub parent: Option<Id>,
    pub label: String,
    pub capacity: Option<u32>,
    #[serde(default)]
    pub attrs: Attrs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: Id,
    pub kind: String,
    pub place: Id,
    pub state: String,
    #[serde(default)]
    pub attrs: Attrs,
    pub since_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub owner: Id,
    pub metric: String,
    pub value: f64,
    pub max: Option<f64>,
}

/// A change a scene asks for. The world checks it; only valid changes become events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    UpsertPlace { place: Place },
    RemovePlace { id: Id },
    Appear { entity: Entity },
    Move { id: Id, to: Id },
    SetState { id: Id, state: String },
    SetLevel { level: Level },
    ClearLevel { owner: Id, metric: String },
    Flash { at: Id, kind: String },
    Gone { id: Id },
    Stale { source: String, since_ms: i64 },
    Fresh { source: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub ts_ms: i64,
    pub change: Change,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub seq: u64,
    pub places: Vec<Place>,
    pub entities: Vec<Entity>,
    pub levels: Vec<Level>,
    pub stale: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    places: BTreeMap<Id, Place>,
    entities: BTreeMap<Id, Entity>,
    levels: BTreeMap<(Id, String), Level>,
    stale: BTreeMap<String, i64>,
}

impl World {
    pub fn place(&self, id: &str) -> Option<&Place> {
        self.places.get(id)
    }

    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn places(&self) -> impl Iterator<Item = &Place> {
        self.places.values()
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn level(&self, owner: &str, metric: &str) -> Option<&Level> {
        self.levels.get(&(owner.to_string(), metric.to_string()))
    }

    /// Applies a change. Returns false when it is invalid or changes nothing:
    /// such changes are dropped and never become events.
    pub fn apply(&mut self, change: &Change, now_ms: i64) -> bool {
        match change {
            Change::UpsertPlace { place } => {
                if self.places.get(&place.id) == Some(place) {
                    return false;
                }
                self.places.insert(place.id.clone(), place.clone());
                true
            }
            Change::RemovePlace { id } => {
                let occupied = self.entities.values().any(|e| &e.place == id);
                let has_children = self.places.values().any(|p| p.parent.as_ref() == Some(id));
                if occupied || has_children || self.places.remove(id).is_none() {
                    return false;
                }
                self.levels.retain(|(owner, _), _| owner != id);
                true
            }
            Change::Appear { entity } => {
                if self.entities.contains_key(&entity.id) || !self.places.contains_key(&entity.place) {
                    return false;
                }
                self.entities.insert(entity.id.clone(), entity.clone());
                true
            }
            Change::Move { id, to } => {
                if !self.places.contains_key(to) {
                    return false;
                }
                match self.entities.get_mut(id) {
                    Some(e) if &e.place != to => {
                        e.place = to.clone();
                        e.since_ms = now_ms;
                        true
                    }
                    _ => false,
                }
            }
            Change::SetState { id, state } => match self.entities.get_mut(id) {
                Some(e) if &e.state != state => {
                    e.state = state.clone();
                    true
                }
                _ => false,
            },
            Change::SetLevel { level } => {
                let known = self.places.contains_key(&level.owner) || self.entities.contains_key(&level.owner);
                let key = (level.owner.clone(), level.metric.clone());
                if !known || self.levels.get(&key) == Some(level) {
                    return false;
                }
                self.levels.insert(key, level.clone());
                true
            }
            Change::ClearLevel { owner, metric } => self.levels.remove(&(owner.clone(), metric.clone())).is_some(),
            Change::Flash { at, .. } => self.places.contains_key(at),
            Change::Gone { id } => {
                if self.entities.remove(id).is_none() {
                    return false;
                }
                self.levels.retain(|(owner, _), _| owner != id);
                true
            }
            Change::Stale { source, since_ms } => self.stale.insert(source.clone(), *since_ms) != Some(*since_ms),
            Change::Fresh { source } => self.stale.remove(source).is_some(),
        }
    }

    pub fn snapshot(&self, seq: u64) -> Snapshot {
        Snapshot {
            seq,
            places: self.places.values().cloned().collect(),
            entities: self.entities.values().cloned().collect(),
            levels: self.levels.values().cloned().collect(),
            stale: self.stale.clone(),
        }
    }

    pub fn from_snapshot(s: &Snapshot) -> World {
        World {
            places: s.places.iter().map(|p| (p.id.clone(), p.clone())).collect(),
            entities: s.entities.iter().map(|e| (e.id.clone(), e.clone())).collect(),
            levels: s.levels.iter().map(|l| ((l.owner.clone(), l.metric.clone()), l.clone())).collect(),
            stale: s.stale.clone(),
        }
    }
}
```

Добавить в `src/lib.rs` строку `pub mod world;`.

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cargo test world:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `10 passed`, без предупреждений.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat: world model with validated changes and snapshots

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 3: Шина событий

**Files:**
- Create: `src/world/bus.rs`
- Modify: `src/world/mod.rs` — добавить `pub mod bus;` после doc-комментария

**Interfaces:**
- Consumes: `Change`, `Event` (задача 2).
- Produces: `pub const BUFFER: usize = 10_000;` `pub struct Gap;` `EventBus::new(cap)`, `publish(ts_ms, Change) -> Event`, `last_seq() -> u64`, `since(after) -> Result<Vec<Event>, Gap>`, `subscribe() -> broadcast::Receiver<Event>`.

- [ ] **Step 1: Написать падающие тесты**

`src/world/bus.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn gone(id: &str) -> Change {
        Change::Gone { id: id.into() }
    }

    #[test]
    fn seq_grows_from_one() {
        let bus = EventBus::new(10);
        assert_eq!(bus.last_seq(), 0);
        assert_eq!(bus.publish(5, gone("a")).seq, 1);
        assert_eq!(bus.publish(6, gone("b")).seq, 2);
        assert_eq!(bus.last_seq(), 2);
    }

    #[test]
    fn since_returns_only_newer_events() {
        let bus = EventBus::new(10);
        for id in ["a", "b", "c"] {
            bus.publish(0, gone(id));
        }
        let seqs: Vec<u64> = bus.since(1).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![2, 3]);
        assert!(bus.since(3).unwrap().is_empty());
        assert_eq!(bus.since(0).unwrap().len(), 3);
    }

    #[test]
    fn dropped_events_are_a_gap() {
        let bus = EventBus::new(3);
        for id in ["a", "b", "c", "d", "e"] {
            bus.publish(0, gone(id));
        }
        assert_eq!(bus.since(1), Err(Gap), "event 2 is no longer buffered");
        let seqs: Vec<u64> = bus.since(2).unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![3, 4, 5]);
    }

    #[test]
    fn a_client_from_the_future_is_a_gap() {
        let bus = EventBus::new(10);
        bus.publish(0, gone("a"));
        assert_eq!(bus.since(5), Err(Gap), "server restarted and lost its numbering");
    }

    #[test]
    fn empty_bus_has_no_gap_at_zero() {
        assert_eq!(EventBus::new(10).since(0), Ok(vec![]));
    }

    #[tokio::test]
    async fn subscribers_receive_published_events() {
        let bus = EventBus::new(10);
        let mut rx = bus.subscribe();
        bus.publish(7, gone("a"));
        let ev = rx.recv().await.unwrap();
        assert_eq!((ev.seq, ev.ts_ms), (1, 7));
    }
}
```

- [ ] **Step 2: Запустить — тесты падают**

Run: `cargo test world::bus`
Expected: ошибка компиляции `cannot find struct 'EventBus'` (после добавления `pub mod bus;` в `world/mod.rs`).

- [ ] **Step 3: Реализовать шину**

Добавить в начало `src/world/bus.rs`:

```rust
//! Numbers events, keeps the last N for clients catching up, and fans them out live.

use super::{Change, Event};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// Events kept for clients that reconnect (spec §7).
pub const BUFFER: usize = 10_000;

/// The asked-for events are no longer buffered (or never existed): take a fresh snapshot.
#[derive(Debug, PartialEq, Eq)]
pub struct Gap;

pub struct EventBus {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Event>,
}

struct Inner {
    last_seq: u64,
    buffer: VecDeque<Event>,
    cap: usize,
}

impl EventBus {
    pub fn new(cap: usize) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { inner: Mutex::new(Inner { last_seq: 0, buffer: VecDeque::new(), cap }), tx }
    }

    pub fn publish(&self, ts_ms: i64, change: Change) -> Event {
        let mut inner = self.inner.lock().expect("bus lock is never poisoned: no panics inside");
        inner.last_seq += 1;
        let ev = Event { seq: inner.last_seq, ts_ms, change };
        if inner.buffer.len() == inner.cap {
            inner.buffer.pop_front();
        }
        inner.buffer.push_back(ev.clone());
        // Sent under the lock so subscribers see events in seq order; no subscribers is fine.
        let _ = self.tx.send(ev.clone());
        ev
    }

    pub fn last_seq(&self) -> u64 {
        self.inner.lock().expect("bus lock is never poisoned: no panics inside").last_seq
    }

    /// Events with seq > `after`.
    pub fn since(&self, after: u64) -> Result<Vec<Event>, Gap> {
        let inner = self.inner.lock().expect("bus lock is never poisoned: no panics inside");
        if after > inner.last_seq {
            return Err(Gap);
        }
        let oldest = inner.buffer.front().map_or(inner.last_seq + 1, |e| e.seq);
        if after + 1 < oldest {
            return Err(Gap);
        }
        Ok(inner.buffer.iter().filter(|e| e.seq > after).cloned().collect())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}
```

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cargo test world:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `16 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/world/
git commit -m "feat: event bus with a replay buffer and gap detection

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 4: Трейт сцены и runtime

**Files:**
- Create: `src/scene.rs`
- Modify: `src/lib.rs` — добавить `pub mod scene;`

**Interfaces:**
- Consumes: `World`, `Change`, `Snapshot`, `EventBus`, `BUFFER`.
- Produces:
  - `trait Scene: Send + 'static { type Input: Send + 'static; fn id(&self) -> &'static str; fn title(&self) -> &'static str; fn reduce(&mut self, world: &World, input: Self::Input, now_ms: i64) -> Vec<Change>; fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change>; }`
  - `struct SceneState { world: World, seq: u64 }`
  - `#[derive(Clone)] struct SceneHandle { id: &'static str, title: &'static str, state: Arc<RwLock<SceneState>>, bus: Arc<EventBus> }` с методами `new(id, title, cap)`, `snapshot() -> Snapshot`, `commit(Vec<Change>, now_ms) -> usize`
  - `fn spawn<S: Scene>(scene: S, tick_every: Duration) -> (SceneHandle, mpsc::Sender<S::Input>)`
  - `fn now_ms() -> i64`

- [ ] **Step 1: Написать падающие тесты**

`src/scene.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Attrs, Entity, Place};

    fn zone(id: &str) -> Change {
        Change::UpsertPlace {
            place: Place { id: id.into(), kind: "zone".into(), parent: None, label: id.into(), capacity: None, attrs: Attrs::new() },
        }
    }

    /// Input: a place id to create. Tick: creates "ticked" once.
    struct Toy;

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
        let (h, tx) = spawn(Toy, Duration::from_millis(20));
        assert_eq!((h.id, h.title), ("toy", "Toy"));
        tx.send("queue".into()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        let snap = h.snapshot();
        let ids: Vec<&str> = snap.places.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"queue") && ids.contains(&"ticked"), "{ids:?}");
    }
}
```

- [ ] **Step 2: Запустить — тесты падают**

Run: `cargo test scene::`
Expected: ошибки компиляции `cannot find trait 'Scene'`.

- [ ] **Step 3: Реализовать**

Добавить в начало `src/scene.rs`:

```rust
//! A scene turns source inputs into world changes. The runtime owns the scene's
//! world, applies the changes and publishes the valid ones as events.

use crate::world::bus::{EventBus, BUFFER};
use crate::world::{Change, Snapshot, World};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

pub trait Scene: Send + 'static {
    type Input: Send + 'static;
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// Pure: reads the world, returns the changes. Must not block.
    fn reduce(&mut self, world: &World, input: Self::Input, now_ms: i64) -> Vec<Change>;
    /// Time-based changes: timeouts, cleanup. Called about once a second.
    fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change>;
}

#[derive(Default)]
pub struct SceneState {
    pub world: World,
    pub seq: u64,
}

#[derive(Clone)]
pub struct SceneHandle {
    pub id: &'static str,
    pub title: &'static str,
    pub state: Arc<RwLock<SceneState>>,
    pub bus: Arc<EventBus>,
}

impl SceneHandle {
    pub fn new(id: &'static str, title: &'static str, cap: usize) -> Self {
        Self { id, title, state: Arc::new(RwLock::new(SceneState::default())), bus: Arc::new(EventBus::new(cap)) }
    }

    /// World and seq read under one lock, so the snapshot matches its seq exactly.
    pub fn snapshot(&self) -> Snapshot {
        let st = self.state.read().expect("scene lock is never poisoned: no panics inside");
        st.world.snapshot(st.seq)
    }

    /// Applies changes in order; the valid ones become events. Returns how many.
    pub fn commit(&self, changes: Vec<Change>, now_ms: i64) -> usize {
        let mut st = self.state.write().expect("scene lock is never poisoned: no panics inside");
        let mut applied = 0;
        for change in changes {
            if st.world.apply(&change, now_ms) {
                st.seq = self.bus.publish(now_ms, change).seq;
                applied += 1;
            }
        }
        applied
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

pub fn spawn<S: Scene>(mut scene: S, tick_every: Duration) -> (SceneHandle, mpsc::Sender<S::Input>) {
    let handle = SceneHandle::new(scene.id(), scene.title(), BUFFER);
    let (tx, mut rx) = mpsc::channel::<S::Input>(256);
    let h = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tick_every);
        loop {
            tokio::select! {
                input = rx.recv() => {
                    let Some(input) = input else { break };
                    let now = now_ms();
                    let changes = {
                        let st = h.state.read().expect("scene lock is never poisoned: no panics inside");
                        scene.reduce(&st.world, input, now)
                    };
                    h.commit(changes, now);
                }
                _ = tick.tick() => {
                    let now = now_ms();
                    let changes = {
                        let st = h.state.read().expect("scene lock is never poisoned: no panics inside");
                        scene.tick(&st.world, now)
                    };
                    h.commit(changes, now);
                }
            }
        }
    });
    (handle, tx)
}
```

Добавить в `src/lib.rs` строку `pub mod scene;`.

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cargo test scene:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `3 passed`.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat: scene trait and runtime with consistent snapshots

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 5: Сцена CI — входы, настройки, подбор раннера

**Files:**
- Create: `src/scenes/mod.rs`, `src/scenes/ci/mod.rs`, `src/scenes/ci/input.rs`, `src/scenes/ci/settings.rs`, `src/scenes/ci/matching.rs`
- Modify: `src/lib.rs` — добавить `pub mod scenes;`

**Interfaces:**
- Produces (`scenes::ci::input`):
  - `enum JobStatus { Pending, Running, Success, Failed, Canceled }` + `as_str(self) -> &'static str`
  - `struct JobInfo { id: u64, name, stage, status: JobStatus, tags: Vec<String>, project: String, pipeline: u64, runner_id: Option<u64> }`
  - `struct JobsUpdate { active: Vec<JobInfo>, recent_finished: Vec<JobInfo> }`
  - `enum RunnerScope { Instance, Group { full_path }, Project { full_paths } }`
  - `struct RunnerInfo { id: u64, description, online: bool, tags, run_untagged: bool, scope: RunnerScope }`
  - `struct AgentReport { agent, host_label, processes: Vec<ProcessReport>, disk: Option<DiskUsage> }`
  - `struct ProcessReport { key, concurrent: Option<u32>, runner_ids: Vec<u64>, limits: BTreeMap<u64, u32>, slots: Vec<SlotReport> }`
  - `struct SlotReport { slot: u32, job_id: u64, runner_id: u64, mem_bytes: u64, mem_limit_bytes: Option<u64>, has_service: bool }`
  - `struct DiskUsage { used_bytes: u64, total_bytes: u64 }`
  - `struct ContainerExit { agent, job_id: u64, oom: bool, exit_code: i64 }`
  - `enum CiInput { Jobs(JobsUpdate), Runners(Vec<RunnerInfo>), Agent(AgentReport), AgentExit(ContainerExit), AgentGone { agent }, SourceStale { source, since_ms }, SourceFresh { source } }`
- Produces (`scenes::ci::settings`): `CiSettings { exits: Vec<ExitRule>, max_projects: usize, default_capacity: u32, capacities: BTreeMap<String, u32>, finished_linger_ms: i64, missing_grace_ms: i64, project_idle_ms: i64 }` + `compile_exits() -> anyhow::Result<Vec<Exit>>`; `ExitRule { label, stage: Option<String>, name: Option<String> }`; `Exit { id, label }` + `matches(&JobInfo) -> bool`.
- Produces (`scenes::ci::matching`): `runner_can_take(&JobInfo, &RunnerInfo) -> bool`.

- [ ] **Step 1: Написать падающие тесты**

`src/scenes/ci/matching.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::{JobStatus, RunnerScope};

    fn job(project: &str, tags: &[&str]) -> JobInfo {
        JobInfo {
            id: 1, name: "test".into(), stage: "test".into(), status: JobStatus::Pending,
            tags: tags.iter().map(|t| t.to_string()).collect(), project: project.into(), pipeline: 1, runner_id: None,
        }
    }
    fn runner(tags: &[&str], untagged: bool, scope: RunnerScope) -> RunnerInfo {
        RunnerInfo { id: 1, description: "r".into(), online: true, tags: tags.iter().map(|t| t.to_string()).collect(), run_untagged: untagged, scope }
    }

    #[test]
    fn job_tags_must_all_be_on_the_runner() {
        let r = runner(&["ops", "heavy"], false, RunnerScope::Instance);
        assert!(runner_can_take(&job("a/b", &["ops"]), &r));
        assert!(runner_can_take(&job("a/b", &["ops", "heavy"]), &r));
        assert!(!runner_can_take(&job("a/b", &["ops", "gpu"]), &r));
    }

    #[test]
    fn untagged_jobs_need_run_untagged() {
        assert!(runner_can_take(&job("a/b", &[]), &runner(&["ops"], true, RunnerScope::Instance)));
        assert!(!runner_can_take(&job("a/b", &[]), &runner(&["ops"], false, RunnerScope::Instance)));
    }

    #[test]
    fn group_runner_takes_its_group_and_subgroups_only() {
        let r = runner(&[], true, RunnerScope::Group { full_path: "studio".into() });
        assert!(runner_can_take(&job("studio/app", &[]), &r));
        assert!(runner_can_take(&job("studio/web/app", &[]), &r));
        assert!(!runner_can_take(&job("studio-old/app", &[]), &r), "prefix is not a group");
        assert!(!runner_can_take(&job("other/app", &[]), &r));
    }

    #[test]
    fn project_runner_takes_listed_projects_only() {
        let r = runner(&[], true, RunnerScope::Project { full_paths: vec!["a/x".into()] });
        assert!(runner_can_take(&job("a/x", &[]), &r));
        assert!(!runner_can_take(&job("a/y", &[]), &r));
    }
}
```

`src/scenes/ci/settings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::JobStatus;

    fn job(stage: &str, name: &str) -> JobInfo {
        JobInfo { id: 1, name: name.into(), stage: stage.into(), status: JobStatus::Success, tags: vec![], project: "a/b".into(), pipeline: 1, runner_id: None }
    }

    #[test]
    fn defaults_come_from_an_empty_object() {
        let s: CiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.max_projects, 8);
        assert_eq!(s.default_capacity, 4);
        assert_eq!(s.finished_linger_ms, 3_000);
        assert_eq!(s.missing_grace_ms, 120_000);
        assert_eq!(s.project_idle_ms, 6 * 3600 * 1000);
        assert!(s.exits.is_empty());
    }

    #[test]
    fn exit_patterns_match_whole_values() {
        let s = CiSettings {
            exits: vec![
                ExitRule { label: "registry".into(), stage: Some("build".into()), name: None },
                ExitRule { label: "prod".into(), stage: Some("deploy".into()), name: Some("deploy:prod|release".into()) },
            ],
            ..CiSettings::default()
        };
        let exits = s.compile_exits().unwrap();
        assert_eq!(exits[0].id, "exit:registry");
        assert!(exits[0].matches(&job("build", "anything")));
        assert!(!exits[0].matches(&job("prebuild", "x")), "whole value, not a substring");
        assert!(exits[1].matches(&job("deploy", "release")));
        assert!(!exits[1].matches(&job("deploy", "deploy:stage")));
    }

    #[test]
    fn a_rule_without_patterns_never_matches() {
        let s = CiSettings { exits: vec![ExitRule { label: "x".into(), stage: None, name: None }], ..CiSettings::default() };
        assert!(!s.compile_exits().unwrap()[0].matches(&job("build", "build")));
    }

    #[test]
    fn a_broken_pattern_is_an_error() {
        let s = CiSettings { exits: vec![ExitRule { label: "x".into(), stage: Some("(".into()), name: None }], ..CiSettings::default() };
        assert!(s.compile_exits().is_err());
    }
}
```

`src/scenes/ci/input.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_report_round_trips_with_numeric_map_keys() {
        let report = CiInput::Agent(AgentReport {
            agent: "a".into(),
            host_label: "h".into(),
            processes: vec![ProcessReport {
                key: "p".into(), concurrent: Some(5), runner_ids: vec![1, 2], limits: BTreeMap::from([(2, 2)]),
                slots: vec![SlotReport { slot: 0, job_id: 9, runner_id: 1, mem_bytes: 10, mem_limit_bytes: Some(20), has_service: true }],
            }],
            disk: Some(DiskUsage { used_bytes: 1, total_bytes: 2 }),
        });
        let text = serde_json::to_string(&report).unwrap();
        assert!(text.contains(r#""type":"agent""#), "{text}");
        assert_eq!(serde_json::from_str::<CiInput>(&text).unwrap(), report);
    }

    #[test]
    fn status_names() {
        assert_eq!(JobStatus::Pending.as_str(), "pending");
        assert_eq!(JobStatus::Canceled.as_str(), "canceled");
        assert_eq!(serde_json::to_string(&JobStatus::Running).unwrap(), r#""running""#);
    }
}
```

`src/scenes/mod.rs`:

```rust
pub mod ci;
```

`src/scenes/ci/mod.rs`:

```rust
//! The CI scene: GitLab jobs flowing through runner processes and their slots.

pub mod input;
pub mod matching;
pub mod settings;
```

Добавить в `src/lib.rs` строку `pub mod scenes;`.

- [ ] **Step 2: Запустить — тесты падают**

Run: `cargo test scenes::ci`
Expected: ошибки компиляции `cannot find type 'JobInfo'`, `'CiSettings'`, `'CiInput'`.

- [ ] **Step 3: Реализовать входы**

Добавить в начало `src/scenes/ci/input.rs`:

```rust
//! What the CI scene is fed with: GitLab polls and agent reports.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Running => "running",
            JobStatus::Success => "success",
            JobStatus::Failed => "failed",
            JobStatus::Canceled => "canceled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: JobStatus,
    pub tags: Vec<String>,
    /// Full path, e.g. `group/project`.
    pub project: String,
    pub pipeline: u64,
    pub runner_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobsUpdate {
    /// Every pending and running job.
    pub active: Vec<JobInfo>,
    /// The newest finished jobs, to catch finishes between polls.
    pub recent_finished: Vec<JobInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerScope {
    Instance,
    Group { full_path: String },
    Project { full_paths: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunnerInfo {
    pub id: u64,
    pub description: String,
    pub online: bool,
    pub tags: Vec<String>,
    pub run_untagged: bool,
    pub scope: RunnerScope,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    pub agent: String,
    pub host_label: String,
    pub processes: Vec<ProcessReport>,
    pub disk: Option<DiskUsage>,
}

/// One gitlab-runner process: its registrations share its slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessReport {
    pub key: String,
    /// `concurrent` from the runner's metrics, when known.
    pub concurrent: Option<u32>,
    #[serde(default)]
    pub runner_ids: Vec<u64>,
    /// Per-registration `limit`, when known.
    #[serde(default)]
    pub limits: BTreeMap<u64, u32>,
    /// Occupied slots only.
    pub slots: Vec<SlotReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotReport {
    pub slot: u32,
    pub job_id: u64,
    pub runner_id: u64,
    pub mem_bytes: u64,
    pub mem_limit_bytes: Option<u64>,
    pub has_service: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContainerExit {
    pub agent: String,
    pub job_id: u64,
    pub oom: bool,
    pub exit_code: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CiInput {
    Jobs(JobsUpdate),
    Runners(Vec<RunnerInfo>),
    Agent(AgentReport),
    AgentExit(ContainerExit),
    AgentGone { agent: String },
    SourceStale { source: String, since_ms: i64 },
    SourceFresh { source: String },
}
```

- [ ] **Step 4: Реализовать настройки и подбор**

Добавить в начало `src/scenes/ci/settings.rs`:

```rust
//! CI scene settings: exits, folding, capacities, timings.

use super::input::JobInfo;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CiSettings {
    pub exits: Vec<ExitRule>,
    /// Projects with their own place; the rest fold into "other".
    pub max_projects: usize,
    /// Slots of a runner process whose `concurrent` is unknown.
    pub default_capacity: u32,
    /// Per process key, overrides `default_capacity`.
    pub capacities: BTreeMap<String, u32>,
    pub finished_linger_ms: i64,
    pub missing_grace_ms: i64,
    pub project_idle_ms: i64,
}

impl Default for CiSettings {
    fn default() -> Self {
        Self {
            exits: Vec::new(),
            max_projects: 8,
            default_capacity: 4,
            capacities: BTreeMap::new(),
            finished_linger_ms: 3_000,
            missing_grace_ms: 120_000,
            project_idle_ms: 6 * 3600 * 1000,
        }
    }
}

/// A successful job matching the rule flies to the exit. Patterns match whole values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitRule {
    pub label: String,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

pub struct Exit {
    pub id: String,
    pub label: String,
    stage: Option<Regex>,
    name: Option<Regex>,
}

impl Exit {
    pub fn matches(&self, job: &JobInfo) -> bool {
        if self.stage.is_none() && self.name.is_none() {
            return false;
        }
        self.stage.as_ref().is_none_or(|r| r.is_match(&job.stage)) && self.name.as_ref().is_none_or(|r| r.is_match(&job.name))
    }
}

impl CiSettings {
    pub fn compile_exits(&self) -> anyhow::Result<Vec<Exit>> {
        self.exits
            .iter()
            .map(|rule| {
                Ok(Exit {
                    id: format!("exit:{}", rule.label),
                    label: rule.label.clone(),
                    stage: rule.stage.as_deref().map(whole).transpose()?,
                    name: rule.name.as_deref().map(whole).transpose()?,
                })
            })
            .collect()
    }
}

fn whole(pattern: &str) -> anyhow::Result<Regex> {
    Ok(Regex::new(&format!("^(?:{pattern})$"))?)
}
```

Добавить в начало `src/scenes/ci/matching.rs`:

```rust
//! Could this runner ever pick this job up?

use super::input::{JobInfo, RunnerInfo, RunnerScope};

pub fn runner_can_take(job: &JobInfo, runner: &RunnerInfo) -> bool {
    let tags_ok = if job.tags.is_empty() { runner.run_untagged } else { job.tags.iter().all(|t| runner.tags.contains(t)) };
    let scope_ok = match &runner.scope {
        RunnerScope::Instance => true,
        RunnerScope::Group { full_path } => job.project.starts_with(&format!("{full_path}/")),
        RunnerScope::Project { full_paths } => full_paths.iter().any(|p| p == &job.project),
    };
    tags_ok && scope_ok
}
```

- [ ] **Step 5: Запустить — тесты проходят**

Run: `cargo test scenes::ci && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `10 passed`.

- [ ] **Step 6: Commit**

```bash
git add src/
git commit -m "feat(ci): scene inputs, settings with exits, runner matching

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 6: Сцена CI — редьюсер

**Files:**
- Create: `src/scenes/ci/reduce.rs`, `src/scenes/ci/testkit.rs`
- Modify: `src/scenes/ci/mod.rs` — добавить модули

**Interfaces:**
- Consumes: всё из задач 2, 4, 5.
- Produces:
  - `CiScene::new(CiSettings) -> anyhow::Result<CiScene>`; `impl Scene for CiScene { type Input = CiInput; id() == "ci"; title() == "CI" }`
  - id мест: константы `PROJECTS="projects"`, `QUEUE="queue"`, `STUCK="stuck"`, `HOSTS="hosts"`, `NOHOST="nohost"`, `EXITS="exits"`, `SINK="sink"`, `OTHER_PROJECT="project:~other"`; функции `job_id(u64)`, `project_id(&str)`, `host_id(&str)`, `process_id(agent, key)`, `slot_id(agent, key, n)`, `runner_place_id(u64)`.
  - виды мест: `zone`, `queue`, `stuck`, `sink`, `project`, `host`, `process`, `slot`, `exit`. Атрибуты: `project.attrs.path`; `process.attrs.agent`, `process.attrs.online`; у джоба `attrs.name/stage/project/pipeline/tags`.
  - состояния джоба: `pending`, `waiting_limit`, `stuck`, `running`, `success`, `failed`, `canceled`, `oom`. Уровни: `mem` у джоба, `disk` у хоста.
  - `testkit::{Harness, job, running, runner, jobs, agent, process, slot}` для задач 6–7.

Правила редьюсера (spec §4): новый джоб появляется у своего проекта и перелетает в цель; PENDING → `queue` / `stuck` / `queue`+`waiting_limit`; RUNNING → слот из отчёта агента, иначе остаётся в своём слоте, пока агент онлайн, иначе место процесса, иначе `runner:<id>` в `nohost`; завершённый → состояние + перелёт в выход (успех и правило) или в `sink`; через `finished_linger_ms` — `Gone`; пропавший без следа — `Gone` через `missing_grace_ms`; пустой проект без активности `project_idle_ms` — удаляется.

- [ ] **Step 1: Написать тестовую обвязку**

`src/scenes/ci/testkit.rs`:

```rust
//! Test harness for the CI scene: feeds inputs, applies changes to a world.

use super::input::*;
use super::reduce::{job_id, CiScene};
use super::settings::CiSettings;
use crate::scene::Scene;
use crate::world::{Change, World};
use std::collections::BTreeMap;

const GIB: f64 = (1u64 << 30) as f64;

pub struct Harness {
    pub scene: CiScene,
    pub world: World,
    pub now: i64,
}

impl Harness {
    pub fn new(settings: CiSettings) -> Self {
        Self { scene: CiScene::new(settings).unwrap(), world: World::default(), now: 1_000_000 }
    }

    /// Reduces the input and returns the changes that were actually applied.
    pub fn feed(&mut self, input: CiInput) -> Vec<Change> {
        let changes = self.scene.reduce(&self.world, input, self.now);
        self.apply(changes)
    }

    pub fn tick(&mut self, advance_ms: i64) -> Vec<Change> {
        self.now += advance_ms;
        let changes = self.scene.tick(&self.world, self.now);
        self.apply(changes)
    }

    fn apply(&mut self, changes: Vec<Change>) -> Vec<Change> {
        changes.into_iter().filter(|c| self.world.apply(c, self.now)).collect()
    }

    pub fn place_of(&self, job: u64) -> Option<String> {
        self.world.entity(&job_id(job)).map(|e| e.place.clone())
    }

    pub fn state_of(&self, job: u64) -> Option<String> {
        self.world.entity(&job_id(job)).map(|e| e.state.clone())
    }
}

pub fn job(id: u64, project: &str, status: JobStatus, tags: &[&str]) -> JobInfo {
    JobInfo {
        id, name: format!("job{id}"), stage: "test".into(), status,
        tags: tags.iter().map(|t| t.to_string()).collect(), project: project.into(), pipeline: 1, runner_id: None,
    }
}

pub fn running(id: u64, project: &str, runner: u64) -> JobInfo {
    JobInfo { runner_id: Some(runner), ..job(id, project, JobStatus::Running, &[]) }
}

pub fn runner(id: u64, tags: &[&str], untagged: bool) -> RunnerInfo {
    RunnerInfo {
        id, description: format!("runner{id}"), online: true,
        tags: tags.iter().map(|t| t.to_string()).collect(), run_untagged: untagged, scope: RunnerScope::Instance,
    }
}

pub fn jobs(active: Vec<JobInfo>, recent_finished: Vec<JobInfo>) -> CiInput {
    CiInput::Jobs(JobsUpdate { active, recent_finished })
}

pub fn agent(name: &str, host: &str, processes: Vec<ProcessReport>) -> CiInput {
    CiInput::Agent(AgentReport { agent: name.into(), host_label: host.into(), processes, disk: None })
}

pub fn process(key: &str, concurrent: Option<u32>, slots: Vec<SlotReport>) -> ProcessReport {
    ProcessReport { key: key.into(), concurrent, runner_ids: vec![], limits: BTreeMap::new(), slots }
}

pub fn slot(n: u32, job: u64, runner: u64, mem_gib: f64, limit_gib: f64) -> SlotReport {
    SlotReport {
        slot: n, job_id: job, runner_id: runner,
        mem_bytes: (mem_gib * GIB) as u64, mem_limit_bytes: Some((limit_gib * GIB) as u64), has_service: false,
    }
}
```

Заменить `src/scenes/ci/mod.rs` целиком:

```rust
//! The CI scene: GitLab jobs flowing through runner processes and their slots.

pub mod input;
pub mod matching;
mod reduce;
pub mod settings;
#[cfg(test)]
pub(crate) mod testkit;

pub use reduce::*;
```

- [ ] **Step 2: Написать падающие тесты: джобы из GitLab**

`src/scenes/ci/reduce.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::input::*;
    use crate::scenes::ci::settings::{CiSettings, ExitRule};
    use crate::scenes::ci::testkit::*;
    use crate::world::Change;

    fn with_exits() -> CiSettings {
        CiSettings {
            exits: vec![ExitRule { label: "registry".into(), stage: Some("build".into()), name: None }],
            ..CiSettings::default()
        }
    }

    #[test]
    fn new_pending_job_appears_at_its_project_then_queues() {
        let mut h = Harness::new(CiSettings::default());
        let ch = h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        let appear = ch.iter().find_map(|c| match c { Change::Appear { entity } => Some(entity.place.clone()), _ => None });
        assert_eq!(appear.as_deref(), Some("project:g/app"));
        assert!(ch.contains(&Change::Move { id: "job:1".into(), to: QUEUE.into() }));
        assert_eq!(h.state_of(1).as_deref(), Some("pending"));
        assert_eq!(h.world.place("project:g/app").unwrap().label, "app");
    }

    #[test]
    fn job_without_a_matching_runner_is_stuck_until_one_shows_up() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::Runners(vec![runner(1, &["ops"], false)]));
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &["docker"])], vec![]));
        assert_eq!(h.place_of(1).as_deref(), Some(STUCK));
        assert_eq!(h.state_of(1).as_deref(), Some("stuck"));
        h.feed(CiInput::Runners(vec![runner(1, &["ops"], false), runner(2, &["docker"], false)]));
        assert_eq!(h.place_of(1).as_deref(), Some(QUEUE));
        assert_eq!(h.state_of(1).as_deref(), Some("pending"));
    }

    #[test]
    fn offline_runners_and_untagged_rules_count() {
        let mut h = Harness::new(CiSettings::default());
        let mut offline = runner(1, &["docker"], true);
        offline.online = false;
        h.feed(CiInput::Runners(vec![offline, runner(2, &["ops"], false)]));
        h.feed(jobs(vec![job(1, "g/a", JobStatus::Pending, &["docker"]), job(2, "g/a", JobStatus::Pending, &[])], vec![]));
        assert_eq!(h.place_of(1).as_deref(), Some(STUCK), "only an offline runner has the tag");
        assert_eq!(h.place_of(2).as_deref(), Some(STUCK), "nobody runs untagged jobs");
    }

    #[test]
    fn running_job_without_agent_sits_on_its_runner() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::Runners(vec![runner(7, &[], true)]));
        h.feed(jobs(vec![running(1, "g/app", 7)], vec![]));
        assert_eq!(h.place_of(1).as_deref(), Some("runner:7"));
        let place = h.world.place("runner:7").unwrap();
        assert_eq!((place.kind.as_str(), place.parent.as_deref(), place.label.as_str()), ("process", Some(NOHOST), "runner7"));
    }

    #[test]
    fn finished_build_flies_to_its_exit_then_goes() {
        let mut h = Harness::new(with_exits());
        let mut build = running(1, "g/app", 7);
        build.stage = "build".into();
        h.feed(jobs(vec![build.clone()], vec![]));
        build.status = JobStatus::Success;
        h.feed(jobs(vec![], vec![build]));
        assert_eq!(h.place_of(1).as_deref(), Some("exit:registry"));
        assert_eq!(h.state_of(1).as_deref(), Some("success"));
        h.tick(2_999);
        assert!(h.place_of(1).is_some(), "still visible for the linger time");
        h.tick(1);
        assert!(h.place_of(1).is_none());
    }

    #[test]
    fn failed_job_goes_to_the_sink() {
        let mut h = Harness::new(with_exits());
        let mut build = running(1, "g/app", 7);
        build.stage = "build".into();
        h.feed(jobs(vec![build.clone()], vec![]));
        build.status = JobStatus::Failed;
        h.feed(jobs(vec![], vec![build]));
        assert_eq!(h.place_of(1).as_deref(), Some(SINK));
        assert_eq!(h.state_of(1).as_deref(), Some("failed"));
    }

    #[test]
    fn job_finished_between_polls_is_closed_without_running() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        h.feed(jobs(vec![], vec![job(1, "g/app", JobStatus::Success, &[])]));
        assert_eq!(h.place_of(1).as_deref(), Some(SINK));
        assert_eq!(h.state_of(1).as_deref(), Some("success"));
    }

    #[test]
    fn job_first_seen_running_flies_from_its_project() {
        let mut h = Harness::new(CiSettings::default());
        let ch = h.feed(jobs(vec![running(1, "g/app", 7)], vec![]));
        assert!(ch.iter().any(|c| matches!(c, Change::Appear { entity } if entity.place == "project:g/app")));
        assert!(ch.contains(&Change::Move { id: "job:1".into(), to: "runner:7".into() }));
    }

    #[test]
    fn job_finished_never_seen_is_not_shown() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![], vec![job(1, "g/app", JobStatus::Success, &[])]));
        assert!(h.place_of(1).is_none());
    }

    #[test]
    fn vanished_job_is_kept_for_the_grace_period() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        h.feed(jobs(vec![], vec![]));
        h.tick(119_999);
        assert!(h.place_of(1).is_some());
        h.tick(1);
        assert!(h.place_of(1).is_none());
    }

    #[test]
    fn projects_beyond_the_limit_fold_into_other() {
        let mut h = Harness::new(CiSettings { max_projects: 2, ..CiSettings::default() });
        let all = vec![
            job(1, "g/a", JobStatus::Pending, &[]),
            job(2, "g/b", JobStatus::Pending, &[]),
            job(3, "g/c", JobStatus::Pending, &[]),
        ];
        let ch = h.feed(jobs(all, vec![]));
        let born: Vec<(String, String)> = ch
            .iter()
            .filter_map(|c| match c { Change::Appear { entity } => Some((entity.id.clone(), entity.place.clone())), _ => None })
            .collect();
        assert!(born.contains(&("job:3".into(), OTHER_PROJECT.into())), "{born:?}");
        assert_eq!(h.world.place(OTHER_PROJECT).unwrap().label, "other");
    }

    #[test]
    fn idle_empty_project_is_removed() {
        let mut h = Harness::new(CiSettings { project_idle_ms: 10_000, finished_linger_ms: 1_000, ..CiSettings::default() });
        h.feed(jobs(vec![job(1, "g/app", JobStatus::Pending, &[])], vec![]));
        h.feed(jobs(vec![], vec![job(1, "g/app", JobStatus::Canceled, &[])]));
        h.tick(1_000);
        assert!(h.world.place("project:g/app").is_some());
        let ch = h.tick(9_000);
        assert!(ch.contains(&Change::RemovePlace { id: "project:g/app".into() }));
    }

    #[test]
    fn stale_and_fresh_pass_through() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::SourceStale { source: "gitlab".into(), since_ms: 5 });
        assert_eq!(h.world.snapshot(0).stale.get("gitlab"), Some(&5));
        h.feed(CiInput::SourceFresh { source: "gitlab".into() });
        assert!(h.world.snapshot(0).stale.is_empty());
    }

    #[test]
    fn static_zones_exist_after_any_input() {
        let mut h = Harness::new(with_exits());
        h.tick(0);
        for id in [PROJECTS, QUEUE, STUCK, HOSTS, NOHOST, EXITS, SINK, "exit:registry"] {
            assert!(h.world.place(id).is_some(), "{id}");
        }
        assert_eq!(h.world.place("exit:registry").unwrap().parent.as_deref(), Some(EXITS));
    }
}
```

- [ ] **Step 3: Написать падающие тесты: агенты, лимиты, OOM**

Дописать в тот же модуль `tests` в `src/scenes/ci/reduce.rs`:

```rust
    fn heavy(id: u64) -> JobInfo {
        JobInfo { tags: vec!["heavy".into()], ..running(id, "g/app", 2) }
    }

    fn shared(slots: Vec<SlotReport>) -> CiInput {
        let mut p = process("p", Some(5), slots);
        p.limits.insert(2, 2);
        agent("a", "host", vec![p])
    }

    #[test]
    fn agent_report_builds_host_process_and_slots() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(3), vec![])]));
        assert_eq!(h.world.place("host:host").unwrap().parent.as_deref(), Some(HOSTS));
        let p = h.world.place("process:a/p").unwrap();
        assert_eq!((p.capacity, p.parent.as_deref()), (Some(3), Some("host:host")));
        assert_eq!(p.attrs.get("agent"), Some(&serde_json::json!("a")));
        assert_eq!(p.attrs.get("online"), Some(&serde_json::json!(true)));
        for n in 0..3 {
            assert_eq!(h.world.place(&format!("slot:a/p/{n}")).unwrap().parent.as_deref(), Some("process:a/p"));
        }
        assert!(h.world.place("slot:a/p/3").is_none());
    }

    #[test]
    fn running_job_lands_in_its_slot_with_memory() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(1, 5, 1, 1.5, 3.0)])]));
        h.feed(jobs(vec![running(5, "g/app", 1)], vec![]));
        assert_eq!(h.place_of(5).as_deref(), Some("slot:a/p/1"));
        let mem = h.world.level("job:5", "mem").unwrap();
        assert_eq!(mem.max, Some(3.0 * (1u64 << 30) as f64));
        assert!((mem.value / mem.max.unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn agent_ahead_of_gitlab_creates_no_phantom_job() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(1, 99, 1, 0.5, 3.0)])]));
        assert!(h.world.entity("job:99").is_none());
        let ch = h.feed(jobs(vec![running(99, "g/app", 1)], vec![]));
        assert!(ch.iter().any(|c| matches!(c, Change::Appear { entity } if entity.place == "project:g/app")));
        assert_eq!(h.place_of(99).as_deref(), Some("slot:a/p/1"));
    }

    #[test]
    fn capacity_comes_from_settings_and_grows_to_fit_seen_slots() {
        let mut settings = CiSettings::default();
        settings.capacities.insert("q".into(), 6);
        let mut h = Harness::new(settings);
        h.feed(agent("a", "host", vec![process("p", None, vec![]), process("q", None, vec![])]));
        assert_eq!(h.world.place("process:a/p").unwrap().capacity, Some(4), "default");
        assert_eq!(h.world.place("process:a/q").unwrap().capacity, Some(6), "from settings");
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(3, 8, 1, 1.0, 3.0)])]));
        assert_eq!(h.world.place("process:a/p").unwrap().capacity, Some(4), "slot #3 needs 4 slots");
        h.feed(jobs(vec![running(8, "g/app", 1)], vec![]));
        assert_eq!(h.place_of(8).as_deref(), Some("slot:a/p/3"));
    }

    #[test]
    fn waits_for_the_runner_limit_while_a_slot_is_free() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(CiInput::Runners(vec![runner(2, &["heavy"], false)]));
        h.feed(shared(vec![slot(0, 1, 2, 1.0, 2.0), slot(1, 2, 2, 1.0, 2.0)]));
        let waiting = job(3, "g/app", JobStatus::Pending, &["heavy"]);
        h.feed(jobs(vec![heavy(1), heavy(2), waiting.clone()], vec![]));
        assert_eq!(h.state_of(3).as_deref(), Some("waiting_limit"));
        assert_eq!(h.place_of(3).as_deref(), Some(QUEUE));
        let done = JobInfo { status: JobStatus::Success, ..heavy(1) };
        h.feed(shared(vec![slot(1, 2, 2, 1.0, 2.0)]));
        h.feed(jobs(vec![heavy(2), waiting], vec![done]));
        assert_eq!(h.state_of(3).as_deref(), Some("pending"));
    }

    #[test]
    fn oom_flashes_the_slot_and_the_state_sticks() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(0, 7, 1, 2.9, 3.0)])]));
        h.feed(jobs(vec![running(7, "g/app", 1)], vec![]));
        let ch = h.feed(CiInput::AgentExit(ContainerExit { agent: "a".into(), job_id: 7, oom: true, exit_code: 137 }));
        assert!(ch.contains(&Change::Flash { at: "slot:a/p/0".into(), kind: "oom".into() }));
        assert_eq!(h.state_of(7).as_deref(), Some("oom"));
        let failed = JobInfo { status: JobStatus::Failed, ..running(7, "g/app", 1) };
        h.feed(jobs(vec![], vec![failed]));
        assert_eq!(h.state_of(7).as_deref(), Some("oom"));
        assert_eq!(h.place_of(7).as_deref(), Some(SINK));
        assert!(h.world.level("job:7", "mem").is_none());
    }

    #[test]
    fn job_dropped_by_the_agent_keeps_its_slot_until_gitlab_finishes_it() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(0, 7, 1, 1.0, 3.0)])]));
        h.feed(jobs(vec![running(7, "g/app", 1)], vec![]));
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![])]));
        assert_eq!(h.place_of(7).as_deref(), Some("slot:a/p/0"), "no flicker to the process row");
    }

    #[test]
    fn agent_gone_greys_the_process_and_moves_its_jobs_there() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(0, 7, 1, 1.0, 3.0)])]));
        h.feed(jobs(vec![running(7, "g/app", 1)], vec![]));
        h.feed(CiInput::AgentGone { agent: "a".into() });
        assert_eq!(h.world.place("process:a/p").unwrap().attrs.get("online"), Some(&serde_json::json!(false)));
        assert_eq!(h.place_of(7).as_deref(), Some("process:a/p"));
        h.feed(agent("a", "host", vec![process("p", Some(2), vec![slot(0, 7, 1, 1.0, 3.0)])]));
        assert_eq!(h.world.place("process:a/p").unwrap().attrs.get("online"), Some(&serde_json::json!(true)));
        assert_eq!(h.place_of(7).as_deref(), Some("slot:a/p/0"));
    }

    #[test]
    fn two_agents_on_one_host_share_the_host() {
        let mut h = Harness::new(CiSettings::default());
        h.feed(agent("a", "box", vec![process("p", Some(1), vec![])]));
        h.feed(agent("b", "box", vec![process("q", Some(1), vec![])]));
        assert_eq!(h.world.places().filter(|p| p.kind == "host").count(), 1);
        assert_eq!(h.world.place("process:b/q").unwrap().parent.as_deref(), Some("host:box"));
    }

    #[test]
    fn disk_level_lands_on_the_host() {
        let mut h = Harness::new(CiSettings::default());
        let report = AgentReport { agent: "a".into(), host_label: "box".into(), processes: vec![], disk: Some(DiskUsage { used_bytes: 30, total_bytes: 100 }) };
        h.feed(CiInput::Agent(report));
        let disk = h.world.level("host:box", "disk").unwrap();
        assert_eq!((disk.value, disk.max), (30.0, Some(100.0)));
    }
```

- [ ] **Step 4: Запустить — тесты падают**

Run: `cargo test scenes::ci::reduce`
Expected: ошибки компиляции `cannot find struct 'CiScene'`.

- [ ] **Step 5: Реализовать редьюсер**

Добавить в начало `src/scenes/ci/reduce.rs`:

```rust
//! The CI scene reducer: turns GitLab polls and agent reports into world changes.

use super::input::{AgentReport, CiInput, ContainerExit, JobInfo, JobStatus, JobsUpdate, ProcessReport, RunnerInfo, SlotReport};
use super::matching::runner_can_take;
use super::settings::{CiSettings, Exit};
use crate::scene::Scene;
use crate::world::{Attrs, Change, Entity, Level, Place, World};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub const PROJECTS: &str = "projects";
pub const QUEUE: &str = "queue";
pub const STUCK: &str = "stuck";
pub const HOSTS: &str = "hosts";
pub const NOHOST: &str = "nohost";
pub const EXITS: &str = "exits";
pub const SINK: &str = "sink";
pub const OTHER_PROJECT: &str = "project:~other";

pub fn job_id(id: u64) -> String {
    format!("job:{id}")
}
pub fn project_id(path: &str) -> String {
    format!("project:{path}")
}
pub fn host_id(label: &str) -> String {
    format!("host:{label}")
}
pub fn process_id(agent: &str, key: &str) -> String {
    format!("process:{agent}/{key}")
}
pub fn slot_id(agent: &str, key: &str, n: u32) -> String {
    format!("slot:{agent}/{key}/{n}")
}
pub fn runner_place_id(id: u64) -> String {
    format!("runner:{id}")
}

fn place(id: impl Into<String>, kind: &str, parent: Option<&str>, label: impl Into<String>, capacity: Option<u32>, attrs: Attrs) -> Place {
    Place { id: id.into(), kind: kind.into(), parent: parent.map(str::to_string), label: label.into(), capacity, attrs }
}

fn attrs(value: serde_json::Value) -> Attrs {
    serde_json::from_value(value).expect("callers pass JSON objects")
}

fn job_attrs(job: &JobInfo) -> Attrs {
    attrs(json!({ "name": job.name, "stage": job.stage, "project": job.project, "pipeline": job.pipeline, "tags": job.tags }))
}

/// The agent of a slot id `slot:<agent>/<key>/<n>`.
fn slot_agent(slot: &str) -> Option<&str> {
    slot.strip_prefix("slot:")?.split('/').next()
}

pub struct CiScene {
    settings: CiSettings,
    exits: Vec<Exit>,
    runners: Vec<RunnerInfo>,
    runners_known: bool,
    active: BTreeMap<u64, JobInfo>,
    /// Projects with their own place → last time they had an active job.
    projects: BTreeMap<String, i64>,
    /// Latest report per agent; offline agents keep theirs so processes stay known.
    agents: BTreeMap<String, AgentReport>,
    offline: BTreeSet<String>,
    oom: BTreeSet<u64>,
    finished: BTreeMap<u64, i64>,
    missing: BTreeMap<u64, i64>,
}

type SlotMap<'a> = BTreeMap<u64, (String, &'a SlotReport)>;

impl CiScene {
    pub fn new(settings: CiSettings) -> anyhow::Result<Self> {
        let exits = settings.compile_exits()?;
        Ok(Self {
            settings,
            exits,
            runners: Vec::new(),
            runners_known: false,
            active: BTreeMap::new(),
            projects: BTreeMap::new(),
            agents: BTreeMap::new(),
            offline: BTreeSet::new(),
            oom: BTreeSet::new(),
            finished: BTreeMap::new(),
            missing: BTreeMap::new(),
        })
    }

    fn ensure_static(&self, world: &World) -> Vec<Change> {
        let mut zones = vec![
            place(PROJECTS, "zone", None, "projects", None, Attrs::new()),
            place(QUEUE, "queue", None, "queue", None, Attrs::new()),
            place(STUCK, "stuck", None, "no runner", None, Attrs::new()),
            place(HOSTS, "zone", None, "runners", None, Attrs::new()),
            place(NOHOST, "zone", None, "runners without agent", None, Attrs::new()),
            place(EXITS, "zone", None, "exits", None, Attrs::new()),
            place(SINK, "sink", None, "", None, Attrs::new()),
        ];
        for exit in &self.exits {
            zones.push(place(exit.id.clone(), "exit", Some(EXITS), exit.label.clone(), None, Attrs::new()));
        }
        zones.into_iter().filter(|p| world.place(&p.id).is_none()).map(|place| Change::UpsertPlace { place }).collect()
    }

    /// The place a project's jobs are born in; folds overflow into "other".
    fn project_place(&mut self, world: &World, path: &str, now: i64, out: &mut Vec<Change>) -> String {
        if self.projects.contains_key(path) || self.projects.len() < self.settings.max_projects {
            self.projects.insert(path.to_string(), now);
            let id = project_id(path);
            if world.place(&id).is_none() {
                let label = path.rsplit('/').next().unwrap_or(path);
                out.push(Change::UpsertPlace { place: place(id.clone(), "project", Some(PROJECTS), label, None, attrs(json!({ "path": path }))) });
            }
            id
        } else {
            if world.place(OTHER_PROJECT).is_none() {
                out.push(Change::UpsertPlace { place: place(OTHER_PROJECT, "project", Some(PROJECTS), "other", None, attrs(json!({ "path": "~other" }))) });
            }
            OTHER_PROJECT.to_string()
        }
    }

    fn capacity(&self, p: &ProcessReport) -> u32 {
        let declared = p.concurrent.or_else(|| self.settings.capacities.get(&p.key).copied()).unwrap_or(self.settings.default_capacity);
        let seen = p.slots.iter().map(|s| s.slot + 1).max().unwrap_or(0);
        declared.max(seen)
    }

    fn process_of_runner(&self, runner: u64) -> Option<(&str, &ProcessReport)> {
        self.agents.iter().find_map(|(agent, report)| {
            report
                .processes
                .iter()
                .find(|p| p.runner_ids.contains(&runner) || p.slots.iter().any(|s| s.runner_id == runner))
                .map(|p| (agent.as_str(), p))
        })
    }

    fn slot_map(&self) -> SlotMap<'_> {
        let mut map = BTreeMap::new();
        for (agent, report) in self.agents.iter().filter(|(agent, _)| !self.offline.contains(*agent)) {
            for p in &report.processes {
                for s in &p.slots {
                    map.insert(s.job_id, (slot_id(agent, &p.key, s.slot), s));
                }
            }
        }
        map
    }

    fn slot_online(&self, slot: &str) -> bool {
        slot_agent(slot).is_some_and(|agent| self.agents.contains_key(agent) && !self.offline.contains(agent))
    }

    /// Every candidate runner is at its `limit` while its process still has a free slot.
    fn limited(&self, runner: &RunnerInfo, running: &BTreeMap<u64, u32>) -> bool {
        let Some((_, p)) = self.process_of_runner(runner.id) else { return false };
        let Some(limit) = p.limits.get(&runner.id) else { return false };
        let busy = running.get(&runner.id).copied().unwrap_or(0);
        busy >= *limit && (p.slots.len() as u32) < self.capacity(p)
    }

    fn target(&self, world: &World, job: &JobInfo, slots: &SlotMap<'_>, running: &BTreeMap<u64, u32>) -> (String, &'static str) {
        match job.status {
            JobStatus::Pending => {
                if !self.runners_known {
                    return (QUEUE.into(), "pending");
                }
                let candidates: Vec<&RunnerInfo> = self.runners.iter().filter(|r| r.online && runner_can_take(job, r)).collect();
                if candidates.is_empty() {
                    (STUCK.into(), "stuck")
                } else if candidates.iter().all(|r| self.limited(r, running)) {
                    (QUEUE.into(), "waiting_limit")
                } else {
                    (QUEUE.into(), "pending")
                }
            }
            JobStatus::Running => {
                if let Some((slot, _)) = slots.get(&job.id) {
                    return (slot.clone(), "running");
                }
                if let Some(e) = world.entity(&job_id(job.id)) {
                    if e.place.starts_with("slot:") && self.slot_online(&e.place) {
                        return (e.place.clone(), "running");
                    }
                }
                match job.runner_id {
                    Some(r) => match self.process_of_runner(r) {
                        Some((agent, p)) => (process_id(agent, &p.key), "running"),
                        None => (runner_place_id(r), "running"),
                    },
                    None => (runner_place_id(0), "running"),
                }
            }
            JobStatus::Success | JobStatus::Failed | JobStatus::Canceled => (SINK.into(), job.status.as_str()),
        }
    }

    /// Puts every active job where it belongs: appear, move, state, memory.
    fn place_jobs(&mut self, world: &World, now: i64, out: &mut Vec<Change>) {
        let mut running: BTreeMap<u64, u32> = BTreeMap::new();
        for j in self.active.values().filter(|j| j.status == JobStatus::Running) {
            if let Some(r) = j.runner_id {
                *running.entry(r).or_default() += 1;
            }
        }
        let plan: Vec<(JobInfo, String, &'static str, Option<(u64, Option<u64>)>)> = {
            let slots = self.slot_map();
            self.active
                .values()
                .map(|job| {
                    let (to, state) = self.target(world, job, &slots, &running);
                    let mem = slots.get(&job.id).map(|(_, s)| (s.mem_bytes, s.mem_limit_bytes));
                    (job.clone(), to, state, mem)
                })
                .collect()
        };
        for (job, to, state, mem) in plan {
            if to.starts_with("runner:") && world.place(&to).is_none() {
                let label = job
                    .runner_id
                    .and_then(|r| self.runners.iter().find(|x| x.id == r))
                    .map_or_else(|| "unknown runner".to_string(), |r| r.description.clone());
                out.push(Change::UpsertPlace { place: place(to.clone(), "process", Some(NOHOST), label, None, Attrs::new()) });
            }
            let eid = job_id(job.id);
            let born_at = self.project_place(world, &job.project, now, out);
            match world.entity(&eid) {
                None => {
                    let entity = Entity { id: eid.clone(), kind: "job".into(), place: born_at.clone(), state: state.into(), attrs: job_attrs(&job), since_ms: now };
                    out.push(Change::Appear { entity });
                    if born_at != to {
                        out.push(Change::Move { id: eid.clone(), to });
                    }
                }
                Some(e) => {
                    if e.place != to {
                        out.push(Change::Move { id: eid.clone(), to });
                    }
                    if e.state != state && !self.oom.contains(&job.id) {
                        out.push(Change::SetState { id: eid.clone(), state: state.into() });
                    }
                }
            }
            if let Some((used, limit)) = mem {
                out.push(Change::SetLevel { level: Level { owner: eid, metric: "mem".into(), value: used as f64, max: limit.map(|v| v as f64) } });
            }
        }
    }

    fn finish(&mut self, job: &JobInfo, now: i64, out: &mut Vec<Change>) {
        let eid = job_id(job.id);
        let state = if self.oom.contains(&job.id) { "oom" } else { job.status.as_str() };
        out.push(Change::SetState { id: eid.clone(), state: state.into() });
        let exit = if job.status == JobStatus::Success { self.exits.iter().find(|x| x.matches(job)).map(|x| x.id.clone()) } else { None };
        out.push(Change::Move { id: eid.clone(), to: exit.unwrap_or_else(|| SINK.into()) });
        out.push(Change::ClearLevel { owner: eid, metric: "mem".into() });
        self.finished.insert(job.id, now);
        self.missing.remove(&job.id);
    }

    fn on_jobs(&mut self, world: &World, update: JobsUpdate, now: i64, out: &mut Vec<Change>) {
        self.active = update
            .active
            .into_iter()
            .filter(|j| matches!(j.status, JobStatus::Pending | JobStatus::Running))
            .map(|j| (j.id, j))
            .collect();
        let active: Vec<u64> = self.active.keys().copied().collect();
        for id in active {
            self.missing.remove(&id);
        }
        self.place_jobs(world, now, out);
        let shown: Vec<u64> = world.entities().filter(|e| e.kind == "job").filter_map(|e| e.id.strip_prefix("job:")?.parse().ok()).collect();
        for id in shown {
            if self.active.contains_key(&id) || self.finished.contains_key(&id) {
                continue;
            }
            match update.recent_finished.iter().find(|f| f.id == id) {
                Some(f) => self.finish(f, now, out),
                None => {
                    self.missing.entry(id).or_insert(now);
                }
            }
        }
    }

    fn on_agent(&mut self, world: &World, report: AgentReport, now: i64, out: &mut Vec<Change>) {
        let host = host_id(&report.host_label);
        out.push(Change::UpsertPlace { place: place(host.clone(), "host", Some(HOSTS), report.host_label.clone(), None, Attrs::new()) });
        for p in &report.processes {
            let cap = self.capacity(p);
            let pid = process_id(&report.agent, &p.key);
            let process_attrs = attrs(json!({ "agent": report.agent, "online": true }));
            out.push(Change::UpsertPlace { place: place(pid.clone(), "process", Some(&host), p.key.clone(), Some(cap), process_attrs) });
            for n in 0..cap {
                out.push(Change::UpsertPlace { place: place(slot_id(&report.agent, &p.key, n), "slot", Some(&pid), format!("#{n}"), None, Attrs::new()) });
            }
            for extra in world.places().filter(|s| s.kind == "slot" && s.parent.as_deref() == Some(pid.as_str())) {
                let n: u32 = extra.id.rsplit('/').next().and_then(|v| v.parse().ok()).unwrap_or(0);
                if n >= cap {
                    out.push(Change::RemovePlace { id: extra.id.clone() });
                }
            }
        }
        if let Some(d) = &report.disk {
            out.push(Change::SetLevel { level: Level { owner: host, metric: "disk".into(), value: d.used_bytes as f64, max: Some(d.total_bytes as f64) } });
        }
        self.offline.remove(&report.agent);
        self.agents.insert(report.agent.clone(), report);
        self.place_jobs(world, now, out);
    }

    fn on_agent_gone(&mut self, world: &World, agent: &str, now: i64, out: &mut Vec<Change>) {
        self.offline.insert(agent.to_string());
        let of_agent = |p: &&Place| p.kind == "process" && p.attrs.get("agent").and_then(|v| v.as_str()) == Some(agent);
        for p in world.places().filter(of_agent) {
            let mut grey = p.clone();
            grey.attrs.insert("online".into(), json!(false));
            out.push(Change::UpsertPlace { place: grey });
        }
        self.place_jobs(world, now, out);
    }

    fn on_exit(&mut self, world: &World, exit: ContainerExit, out: &mut Vec<Change>) {
        if !exit.oom {
            return;
        }
        self.oom.insert(exit.job_id);
        let eid = job_id(exit.job_id);
        if let Some(e) = world.entity(&eid) {
            out.push(Change::Flash { at: e.place.clone(), kind: "oom".into() });
            out.push(Change::SetState { id: eid, state: "oom".into() });
        }
    }
}

impl Scene for CiScene {
    type Input = CiInput;

    fn id(&self) -> &'static str {
        "ci"
    }

    fn title(&self) -> &'static str {
        "CI"
    }

    fn reduce(&mut self, world: &World, input: CiInput, now_ms: i64) -> Vec<Change> {
        let mut out = self.ensure_static(world);
        match input {
            CiInput::Jobs(update) => self.on_jobs(world, update, now_ms, &mut out),
            CiInput::Runners(runners) => {
                self.runners = runners;
                self.runners_known = true;
                self.place_jobs(world, now_ms, &mut out);
            }
            CiInput::Agent(report) => self.on_agent(world, report, now_ms, &mut out),
            CiInput::AgentExit(exit) => self.on_exit(world, exit, &mut out),
            CiInput::AgentGone { agent } => self.on_agent_gone(world, &agent, now_ms, &mut out),
            CiInput::SourceStale { source, since_ms } => out.push(Change::Stale { source, since_ms }),
            CiInput::SourceFresh { source } => out.push(Change::Fresh { source }),
        }
        out
    }

    fn tick(&mut self, world: &World, now_ms: i64) -> Vec<Change> {
        let mut out = self.ensure_static(world);
        let linger = self.settings.finished_linger_ms;
        let done: Vec<u64> = self.finished.iter().filter(|(_, at)| now_ms - **at >= linger).map(|(id, _)| *id).collect();
        for id in done {
            self.finished.remove(&id);
            self.oom.remove(&id);
            out.push(Change::Gone { id: job_id(id) });
        }
        let grace = self.settings.missing_grace_ms;
        let lost: Vec<u64> = self.missing.iter().filter(|(_, at)| now_ms - **at >= grace).map(|(id, _)| *id).collect();
        for id in lost {
            self.missing.remove(&id);
            self.oom.remove(&id);
            out.push(Change::Gone { id: job_id(id) });
        }
        let idle = self.settings.project_idle_ms;
        let quiet: Vec<String> = self.projects.iter().filter(|(_, last)| now_ms - **last >= idle).map(|(p, _)| p.clone()).collect();
        for path in quiet {
            let id = project_id(&path);
            if world.entities().any(|e| e.place == id) {
                continue;
            }
            self.projects.remove(&path);
            out.push(Change::RemovePlace { id });
        }
        out
    }
}
```

- [ ] **Step 6: Запустить — тесты проходят**

Run: `cargo test scenes::ci && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: все тесты `scenes::ci` зелёные: 24 новых (14 про джобы, 10 про агентов) и 10 из задачи 5. Если тест падает — исправлять реализацию, не тест: тесты фиксируют spec §4.

- [ ] **Step 7: Commit**

```bash
git add src/scenes/
git commit -m "feat(ci): reducer for jobs, runners, agents, limits and OOM

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 7: Симулятор

**Files:**
- Create: `src/demo/mod.rs`
- Modify: `src/lib.rs` — добавить `pub mod demo;`

**Interfaces:**
- Consumes: `scenes::ci::{input::*, matching::runner_can_take, settings::{CiSettings, ExitRule}}`, `scene::now_ms`, `testkit::Harness` (в тестах).
- Produces: `demo::settings() -> CiSettings` (выходы `registry` для стадии `build`, `prod` для `deploy`), `demo::runners() -> Vec<RunnerInfo>`, `Simulator::new(seed)`, `Simulator::step(now_ms) -> Vec<CiInput>`, `async fn run(tx: mpsc::Sender<CiInput>, seed: u64)`.

Модель симулятора: проекты `demo/backend`, `demo/frontend`, `demo/mobile`, `demo/legacy`, `demo/docs`; хост `runner-host`; процесс `shared` (агент `agent-shared`, 5 слотов, раннеры 1 `ops`+без тегов и 2 `heavy` с лимитом 2) и процесс `deploy` (агент `agent-deploy`, 1 слот, раннер 3 `deploy`). У `legacy` часть джобов с тегом `docker` — для них раннера нет, через 40 с они отменяются. 7% джобов падают по памяти.

- [ ] **Step 1: Написать падающие тесты**

`src/demo/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::ci::testkit::Harness;
    use crate::world::Change;

    #[test]
    fn fifteen_minutes_of_demo_keep_the_world_consistent() {
        let mut sim = Simulator::new(42);
        let mut h = Harness::new(settings());
        let (mut stuck, mut oom, mut exits) = (false, false, false);
        for _ in 0..900 {
            h.now += 1_000;
            for input in sim.step(h.now) {
                for change in h.feed(input) {
                    match &change {
                        Change::Flash { kind, .. } if kind == "oom" => oom = true,
                        Change::Move { to, .. } if to.starts_with("exit:") => exits = true,
                        _ => {}
                    }
                }
            }
            h.tick(0);
            let mut per_slot: BTreeMap<String, u32> = BTreeMap::new();
            for e in h.world.entities() {
                assert!(h.world.place(&e.place).is_some(), "{} sits in a missing place {}", e.id, e.place);
                stuck |= e.state == "stuck";
                if e.place.starts_with("slot:") && e.state == "running" {
                    *per_slot.entry(e.place.clone()).or_default() += 1;
                }
            }
            assert!(per_slot.values().all(|n| *n <= 1), "two running jobs in one slot: {per_slot:?}");
        }
        assert!(stuck && oom && exits, "stuck={stuck} oom={oom} exits={exits}");
    }

    #[test]
    fn same_seed_same_story() {
        let (mut a, mut b) = (Simulator::new(7), Simulator::new(7));
        for t in 0..60 {
            let now = 1_000 * t;
            assert_eq!(a.step(now), b.step(now));
        }
    }

    #[test]
    fn demo_names_are_generic() {
        let text = serde_json::to_string(&Simulator::new(1).step(0)).unwrap();
        assert!(text.contains("runner-host") && text.contains("agent-shared"));
    }
}
```

- [ ] **Step 2: Запустить — тесты падают**

Run: `cargo test demo::`
Expected: ошибки компиляции `cannot find struct 'Simulator'`.

- [ ] **Step 3: Реализовать симулятор**

Добавить в начало `src/demo/mod.rs`:

```rust
//! Built-in simulator: a fake GitLab with runners and agents, so the CI scene can
//! be shown and developed without any real infrastructure. Names are generic on purpose.

use crate::scenes::ci::input::*;
use crate::scenes::ci::matching::runner_can_take;
use crate::scenes::ci::settings::{CiSettings, ExitRule};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;
use tokio::sync::mpsc;

const GIB: u64 = 1 << 30;
const HOST: &str = "runner-host";
const PROJECTS: [(&str, u32); 5] = [("demo/backend", 30), ("demo/frontend", 25), ("demo/mobile", 20), ("demo/legacy", 10), ("demo/docs", 15)];

struct Proc {
    agent: &'static str,
    key: &'static str,
    concurrent: u32,
    runners: &'static [u64],
    limits: &'static [(u64, u32)],
}

const PROCS: [Proc; 2] = [
    Proc { agent: "agent-shared", key: "shared", concurrent: 5, runners: &[1, 2], limits: &[(2, 2)] },
    Proc { agent: "agent-deploy", key: "deploy", concurrent: 1, runners: &[3], limits: &[] },
];

pub fn settings() -> CiSettings {
    CiSettings {
        exits: vec![
            ExitRule { label: "registry".into(), stage: Some("build".into()), name: None },
            ExitRule { label: "prod".into(), stage: Some("deploy".into()), name: None },
        ],
        ..CiSettings::default()
    }
}

pub fn runners() -> Vec<RunnerInfo> {
    let r = |id: u64, description: &str, tag: &str, untagged: bool| RunnerInfo {
        id,
        description: description.into(),
        online: true,
        tags: vec![tag.into()],
        run_untagged: untagged,
        scope: RunnerScope::Instance,
    };
    vec![r(1, "shared", "ops", true), r(2, "heavy", "heavy", false), r(3, "deploy", "deploy", false)]
}

struct SimJob {
    info: JobInfo,
    slot: Option<(usize, u32)>,
    started: i64,
    ends: i64,
    peak: u64,
    limit: u64,
    oom: bool,
    cancel_at: Option<i64>,
}

pub struct Simulator {
    rng: SmallRng,
    next_id: u64,
    jobs: BTreeMap<u64, SimJob>,
    finished: VecDeque<JobInfo>,
    exits: Vec<ContainerExit>,
    disk: f64,
    steps: u64,
    runners: Vec<RunnerInfo>,
}

impl Simulator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: SmallRng::seed_from_u64(seed),
            next_id: 1,
            jobs: BTreeMap::new(),
            finished: VecDeque::new(),
            exits: Vec::new(),
            disk: 0.55,
            steps: 0,
            runners: runners(),
        }
    }

    /// One second of the fake world. Order matters: agent reports and container
    /// exits come before the GitLab poll, as they would in real life.
    pub fn step(&mut self, now: i64) -> Vec<CiInput> {
        self.spawn(now);
        self.schedule(now);
        self.progress(now);
        let mut out = Vec::new();
        if self.steps % 30 == 0 {
            out.push(CiInput::Runners(self.runners.clone()));
        }
        for (i, p) in PROCS.iter().enumerate() {
            out.push(CiInput::Agent(self.report(i, p, now)));
        }
        out.extend(self.exits.drain(..).map(CiInput::AgentExit));
        out.push(CiInput::Jobs(JobsUpdate {
            active: self.jobs.values().map(|j| j.info.clone()).collect(),
            recent_finished: self.finished.iter().cloned().collect(),
        }));
        self.steps += 1;
        out
    }

    fn spawn(&mut self, now: i64) {
        if !self.rng.random_bool(0.55) {
            return;
        }
        let total: u32 = PROJECTS.iter().map(|(_, w)| w).sum();
        let mut roll = self.rng.random_range(0..total);
        let mut project = PROJECTS[0].0;
        for (name, weight) in PROJECTS {
            if roll < weight {
                project = name;
                break;
            }
            roll -= weight;
        }
        let (name, stage, tags): (&str, &str, Vec<&str>) = match project {
            "demo/mobile" => ("e2e", "test", vec!["heavy"]),
            "demo/legacy" if self.rng.random_bool(0.4) => ("build-image", "build", vec!["docker"]),
            "demo/legacy" | "demo/docs" => ("build", "build", vec![]),
            _ => match self.rng.random_range(0..10) {
                0 => ("deploy", "deploy", vec!["deploy"]),
                1..=3 => ("lint", "lint", vec!["ops"]),
                4..=6 => ("test", "test", vec!["ops"]),
                _ => ("build", "build", vec!["ops"]),
            },
        };
        let id = self.next_id;
        self.next_id += 1;
        let cancel_at = (tags == ["docker"]).then_some(now + 40_000);
        let info = JobInfo {
            id,
            name: name.into(),
            stage: stage.into(),
            status: JobStatus::Pending,
            tags: tags.into_iter().map(String::from).collect(),
            project: project.into(),
            pipeline: id / 3 + 1,
            runner_id: None,
        };
        self.jobs.insert(id, SimJob { info, slot: None, started: 0, ends: 0, peak: 0, limit: 0, oom: false, cancel_at });
    }

    fn free_slot(&self, pi: usize) -> Option<u32> {
        (0..PROCS[pi].concurrent).find(|n| !self.jobs.values().any(|j| j.slot == Some((pi, *n))))
    }

    fn pick(&self, job: &JobInfo) -> Option<(usize, u64)> {
        for r in &self.runners {
            if !runner_can_take(job, r) {
                continue;
            }
            let Some(pi) = PROCS.iter().position(|p| p.runners.contains(&r.id)) else { continue };
            if self.free_slot(pi).is_none() {
                continue;
            }
            if let Some((_, limit)) = PROCS[pi].limits.iter().find(|(rid, _)| *rid == r.id) {
                let busy = self.jobs.values().filter(|j| j.info.status == JobStatus::Running && j.info.runner_id == Some(r.id)).count() as u32;
                if busy >= *limit {
                    continue;
                }
            }
            return Some((pi, r.id));
        }
        None
    }

    fn schedule(&mut self, now: i64) {
        let pending: Vec<u64> = self.jobs.values().filter(|j| j.info.status == JobStatus::Pending).map(|j| j.info.id).collect();
        for id in pending {
            let info = self.jobs[&id].info.clone();
            let Some((pi, runner)) = self.pick(&info) else { continue };
            let slot = self.free_slot(pi).expect("pick() checked that a slot is free");
            let heavy = info.tags.iter().any(|t| t == "heavy");
            let deploy = info.stage == "deploy";
            let duration: i64 = if deploy {
                self.rng.random_range(3_000..6_000)
            } else if heavy {
                self.rng.random_range(15_000..30_000)
            } else {
                self.rng.random_range(5_000..15_000)
            };
            let limit = if heavy { 2 * GIB } else { 3 * GIB };
            let oom = !deploy && self.rng.random_bool(0.07);
            let peak = if oom { limit } else { (limit as f64 * self.rng.random_range(0.35..0.95)) as u64 };
            let j = self.jobs.get_mut(&id).expect("id came from the map");
            j.info.status = JobStatus::Running;
            j.info.runner_id = Some(runner);
            j.slot = Some((pi, slot));
            j.started = now;
            j.ends = now + duration;
            j.limit = limit;
            j.peak = peak;
            j.oom = oom;
        }
    }

    fn progress(&mut self, now: i64) {
        let ids: Vec<u64> = self.jobs.keys().copied().collect();
        for id in ids {
            let j = &self.jobs[&id];
            let status = match j.info.status {
                JobStatus::Running if j.oom && now >= j.started + (j.ends - j.started) * 7 / 10 => {
                    let agent = PROCS[j.slot.expect("running jobs have a slot").0].agent;
                    self.exits.push(ContainerExit { agent: agent.into(), job_id: id, oom: true, exit_code: 137 });
                    Some(JobStatus::Failed)
                }
                JobStatus::Running if now >= j.ends => Some(if self.rng.random_bool(0.05) { JobStatus::Failed } else { JobStatus::Success }),
                JobStatus::Pending if j.cancel_at.is_some_and(|t| now >= t) => Some(JobStatus::Canceled),
                _ => None,
            };
            if let Some(status) = status {
                let mut j = self.jobs.remove(&id).expect("id came from the map");
                j.info.status = status;
                self.finished.push_front(j.info);
                self.finished.truncate(30);
                self.disk += 0.004;
                if self.disk > 0.85 {
                    self.disk = 0.45;
                }
            }
        }
    }

    fn report(&self, pi: usize, p: &Proc, now: i64) -> AgentReport {
        let slots = self
            .jobs
            .values()
            .filter_map(|j| {
                let (jp, n) = j.slot?;
                if jp != pi {
                    return None;
                }
                let progress = ((now - j.started) as f64 / (j.ends - j.started).max(1) as f64).clamp(0.0, 1.0);
                let mem = (j.peak as f64 * (0.3 + 1.4 * progress).min(1.0)) as u64;
                Some(SlotReport {
                    slot: n,
                    job_id: j.info.id,
                    runner_id: j.info.runner_id.unwrap_or(0),
                    mem_bytes: mem,
                    mem_limit_bytes: Some(j.limit),
                    has_service: j.info.stage == "build" || j.info.tags.iter().any(|t| t == "heavy"),
                })
            })
            .collect();
        AgentReport {
            agent: p.agent.into(),
            host_label: HOST.into(),
            processes: vec![ProcessReport {
                key: p.key.into(),
                concurrent: Some(p.concurrent),
                runner_ids: p.runners.to_vec(),
                limits: p.limits.iter().copied().collect(),
                slots,
            }],
            disk: Some(DiskUsage { used_bytes: (self.disk * 80.0 * GIB as f64) as u64, total_bytes: 80 * GIB }),
        }
    }
}

/// Feeds the scene one simulated second per real second until the scene stops.
pub async fn run(tx: mpsc::Sender<CiInput>, seed: u64) {
    let mut sim = Simulator::new(seed);
    let mut every = tokio::time::interval(Duration::from_secs(1));
    loop {
        every.tick().await;
        for input in sim.step(crate::scene::now_ms()) {
            if tx.send(input).await.is_err() {
                return;
            }
        }
    }
}
```

Добавить в `src/lib.rs` строку `pub mod demo;`.

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cargo test demo:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `3 passed`. Если `fifteen_minutes…` падает на инварианте — это ошибка редьюсера или порядка входов: чинить её, а не ослаблять проверку. Если падает только финальная строка (`stuck/oom/exits` ни разу не случились) при seed 42 — поднять вес `demo/legacy` или вероятность OOM и записать причину в коммит.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(demo): deterministic CI simulator

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 8: HTTP API и SSE

**Files:**
- Create: `src/http/mod.rs`, `src/http/static_files.rs`
- Modify: `src/lib.rs` — добавить `pub mod http;`

**Interfaces:**
- Consumes: `SceneHandle` (`id`, `title`, `snapshot()`, `bus`), `Gap`, `Event`.
- Produces: `http::{AppState { scenes: Vec<SceneHandle> }, router(Arc<AppState>) -> Router}`. Маршруты: `GET /healthz`, `GET /api/v1/scenes`, `GET /api/v1/scenes/{id}/snapshot`, `GET /api/v1/scenes/{id}/events?since=N` (или заголовок `Last-Event-ID`; заголовок важнее), всё прочее — вшитый интерфейс; `/api/...` без маршрута — 404.

- [ ] **Step 1: Написать падающие тесты**

`src/http/mod.rs`:

```rust
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
        (router(Arc::new(AppState { scenes: vec![h.clone()] })), h)
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
        assert!(body.contains(r#""status":"ok""#), "{body}");
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

- [ ] **Step 2: Запустить — тесты падают**

Run: `cargo test http::`
Expected: ошибки компиляции `cannot find function 'router'`.

- [ ] **Step 3: Реализовать API**

Добавить в начало `src/http/mod.rs`:

```rust
//! HTTP: scene list, snapshots, the live event stream (SSE), and the built-in UI.

mod static_files;

use crate::scene::SceneHandle;
use crate::world::bus::Gap;
use crate::world::Event;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;

pub struct AppState {
    pub scenes: Vec<SceneHandle>,
}

impl AppState {
    fn scene(&self, id: &str) -> Option<&SceneHandle> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/scenes", get(list_scenes))
        .route("/api/v1/scenes/{id}/snapshot", get(snapshot))
        .route("/api/v1/scenes/{id}/events", get(events))
        .fallback(static_files::serve)
        .with_state(state)
}

async fn healthz(State(s): State<Arc<AppState>>) -> Json<Value> {
    let scenes: Vec<Value> = s.scenes.iter().map(|h| json!({ "id": h.id, "stale": h.snapshot().stale })).collect();
    Json(json!({ "status": "ok", "scenes": scenes }))
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
```

`src/http/static_files.rs`:

```rust
//! The UI built by `web/` (`npm run build`), embedded at compile time.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::RustEmbed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct Assets;

const NOT_BUILT: &str = "<!doctype html><title>lorvaq</title>\
<p>lorvaq: the UI is not built. Run <code>npm ci && npm run build</code> in <code>web/</code>, then rebuild.</p>";

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
        return ([(header::CONTENT_TYPE, mime)], file.data).into_response();
    }
    // Client-side routes fall back to the app shell.
    match Assets::get("index.html") {
        Some(index) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())], index.data).into_response(),
        None => ([(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())], NOT_BUILT).into_response(),
    }
}
```

Добавить в `src/lib.rs` строку `pub mod http;`.

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cargo test http:: && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: `7 passed` (с несобранным `web/dist` тест `ui_is_served…` видит страницу `NOT_BUILT` — в ней есть слово `lorvaq`).

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "feat(http): scenes API, snapshots, SSE with replay and 410 on gaps

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 9: Сборка сервера

**Files:**
- Create: `src/app.rs`, `tests/demo_smoke.rs`
- Modify: `src/lib.rs` — добавить `pub mod app;`; `src/main.rs` — заменить целиком

**Interfaces:**
- Consumes: `CiScene`, `scene::spawn`, `demo::{settings, run}`, `http::{router, AppState}`.
- Produces: `app::demo_scenes() -> anyhow::Result<Vec<SceneHandle>>` (нужен tokio-runtime), `app::serve(SocketAddr, Vec<SceneHandle>) -> anyhow::Result<()>` (graceful по Ctrl-C и SIGTERM).

- [ ] **Step 1: Написать падающий интеграционный тест**

`tests/demo_smoke.rs`:

```rust
use lorvaq::app;
use std::time::Duration;

#[tokio::test]
async fn demo_builds_the_scene_within_seconds() {
    let scenes = app::demo_scenes().unwrap();
    assert_eq!(scenes.len(), 1);
    let ci = scenes[0].clone();
    assert_eq!((ci.id, ci.title), ("ci", "CI"));
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let snap = ci.snapshot();
    assert!(snap.seq > 0, "events were published");
    assert!(snap.places.iter().any(|p| p.kind == "slot"), "agent reports created slots");
    assert!(snap.places.iter().any(|p| p.id == "exit:registry"), "demo exits exist");
}
```

- [ ] **Step 2: Запустить — тест падает**

Run: `cargo test --test demo_smoke`
Expected: ошибка компиляции `could not find 'app' in 'lorvaq'`.

- [ ] **Step 3: Реализовать**

`src/app.rs`:

```rust
//! Wires scenes, sources and HTTP into a running server.

use crate::http::{router, AppState};
use crate::scene::{self, SceneHandle};
use crate::scenes::ci::CiScene;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// The CI scene fed by the simulator. Needs a tokio runtime.
pub fn demo_scenes() -> anyhow::Result<Vec<SceneHandle>> {
    let ci = CiScene::new(crate::demo::settings())?;
    let (handle, tx) = scene::spawn(ci, Duration::from_secs(1));
    tokio::spawn(crate::demo::run(tx, 42));
    Ok(vec![handle])
}

pub async fn serve(listen: SocketAddr, scenes: Vec<SceneHandle>) -> anyhow::Result<()> {
    let app = router(Arc::new(AppState { scenes }));
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "lorvaq is listening");
    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
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
use lorvaq::cli::{Cli, Command};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    match Cli::parse().command {
        Command::Server { demo, listen } => {
            anyhow::ensure!(demo, "this build only runs with --demo: real sources arrive in the next releases");
            app::serve(listen, app::demo_scenes()?).await
        }
    }
}
```

Добавить в `src/lib.rs` строку `pub mod app;`.

- [ ] **Step 4: Запустить — всё проходит, сервер отвечает**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: все тесты зелёные, включая `demo_smoke`.

Затем руками:

```bash
cargo run -- server --demo --listen 127.0.0.1:8080 &
sleep 4
curl -fsS 127.0.0.1:8080/healthz
curl -fsS 127.0.0.1:8080/api/v1/scenes/ci/snapshot | head -c 400; echo
curl -fsSN --max-time 3 '127.0.0.1:8080/api/v1/scenes/ci/events' | head -n 6
kill %1
```

Expected: `{"status":"ok",...}`; снимок с местами `projects`, `queue`, слотами `slot:agent-shared/shared/N`; в потоке строки `id: N` и `data: {"seq":N,...}`; после `kill` в логе `shutting down`.

- [ ] **Step 5: Commit**

```bash
git add src/ tests/
git commit -m "feat: demo server wiring with graceful shutdown

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 10: Веб — каркас, типы, зеркало мира, клиент API

**Files:**
- Create: `web/package.json`, `web/tsconfig.json`, `web/vite.config.ts`, `web/index.html`, `web/src/main.tsx`, `web/src/types.ts`, `web/src/world.ts`, `web/src/world.test.ts`, `web/src/api.ts`

**Interfaces:**
- Produces:
  - `types.ts`: `Id`, `Attrs`, `Place`, `Entity`, `Level`, `Change` (union по `op`), `WorldEvent { seq, ts_ms, change }`, `Snapshot`, `SceneInfo { id, title }` — точное зеркало Rust-типов задачи 2.
  - `world.ts`: `class ClientWorld { seq; places: Map; entities: Map; levels: Map; stale: Map; static fromSnapshot(s); level(owner, metric); apply(ev): Effect }`; `type Effect = {type:"appear",id,at} | {type:"move",id,from,to} | {type:"gone",id,from} | {type:"flash",at,kind} | {type:"none"}`.
  - `api.ts`: `fetchScenes()`, `fetchSnapshot(id)`, `follow(sceneId, onWorld(world, effects), onStatus("live"|"reconnecting")) -> { close() }` — снимок, затем SSE; при любом обрыве или `410` — новый снимок.

- [ ] **Step 1: Создать каркас**

`web/package.json`:

```json
{
  "name": "lorvaq-web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "react": "^19.3.0",
    "react-dom": "^19.3.0"
  },
  "devDependencies": {
    "@types/react": "^19.3.0",
    "@types/react-dom": "^19.3.0",
    "@vitejs/plugin-react": "^6.1.2",
    "typescript": "^7.0.2",
    "vite": "^8.3.4",
    "vitest": "^5.0.3"
  }
}
```

`web/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "types": ["vite/client"],
    "skipLibCheck": true
  },
  "include": ["src"]
}
```

`web/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `npm run dev` talks to `cargo run -- server --demo` on :8080.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api": "http://127.0.0.1:8080", "/healthz": "http://127.0.0.1:8080" } },
});
```

`web/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>lorvaq</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`web/src/main.tsx` (временный — задача 12 заменит его целиком):

```tsx
import { createRoot } from "react-dom/client";

const root = document.getElementById("root");
if (root) createRoot(root).render(<p>lorvaq</p>);
```

`web/src/types.ts`:

```ts
// Mirror of the Rust world types (src/world/mod.rs). Change both sides together.

export type Id = string;
export type Attrs = Record<string, unknown>;

export interface Place {
  id: Id;
  kind: string;
  parent: Id | null;
  label: string;
  capacity: number | null;
  attrs: Attrs;
}

export interface Entity {
  id: Id;
  kind: string;
  place: Id;
  state: string;
  attrs: Attrs;
  since_ms: number;
}

export interface Level {
  owner: Id;
  metric: string;
  value: number;
  max: number | null;
}

export type Change =
  | { op: "upsert_place"; place: Place }
  | { op: "remove_place"; id: Id }
  | { op: "appear"; entity: Entity }
  | { op: "move"; id: Id; to: Id }
  | { op: "set_state"; id: Id; state: string }
  | { op: "set_level"; level: Level }
  | { op: "clear_level"; owner: Id; metric: string }
  | { op: "flash"; at: Id; kind: string }
  | { op: "gone"; id: Id }
  | { op: "stale"; source: string; since_ms: number }
  | { op: "fresh"; source: string };

export interface WorldEvent {
  seq: number;
  ts_ms: number;
  change: Change;
}

export interface Snapshot {
  seq: number;
  places: Place[];
  entities: Entity[];
  levels: Level[];
  stale: Record<string, number>;
}

export interface SceneInfo {
  id: string;
  title: string;
}
```

Run: `cd web && npm install`
Expected: создан `web/package-lock.json`, ошибок нет.

- [ ] **Step 2: Написать падающие тесты зеркала мира**

`web/src/world.test.ts`:

```ts
import { describe, expect, test } from "vitest";
import { ClientWorld } from "./world";
import type { Change, Entity, Place, Snapshot } from "./types";

const place = (id: string, extra: Partial<Place> = {}): Place => ({ id, kind: "zone", parent: null, label: id, capacity: null, attrs: {}, ...extra });
const job = (id: string, at: string): Entity => ({ id, kind: "job", place: at, state: "pending", attrs: {}, since_ms: 0 });
const snap = (): Snapshot => ({ seq: 3, places: [place("q"), place("slot")], entities: [job("j1", "q")], levels: [], stale: {} });
let seq = 3;
const ev = (change: Change) => ({ seq: ++seq, ts_ms: 100, change });

describe("ClientWorld", () => {
  test("starts from a snapshot", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.seq).toBe(3);
    expect(w.entities.get("j1")?.place).toBe("q");
  });

  test("move reports where from and where to", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.apply(ev({ op: "move", id: "j1", to: "slot" }))).toEqual({ type: "move", id: "j1", from: "q", to: "slot" });
    expect(w.entities.get("j1")?.place).toBe("slot");
    expect(w.entities.get("j1")?.since_ms).toBe(100);
    expect(w.seq).toBe(seq);
  });

  test("appear and gone", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.apply(ev({ op: "appear", entity: job("j2", "q") }))).toEqual({ type: "appear", id: "j2", at: "q" });
    w.apply(ev({ op: "set_level", level: { owner: "j2", metric: "mem", value: 1, max: 2 } }));
    expect(w.level("j2", "mem")?.value).toBe(1);
    expect(w.apply(ev({ op: "gone", id: "j2" }))).toEqual({ type: "gone", id: "j2", from: "q" });
    expect(w.entities.has("j2")).toBe(false);
    expect(w.level("j2", "mem")).toBeUndefined();
  });

  test("places, states, levels, flashes, staleness", () => {
    const w = ClientWorld.fromSnapshot(snap());
    w.apply(ev({ op: "upsert_place", place: place("x", { kind: "slot" }) }));
    expect(w.places.get("x")?.kind).toBe("slot");
    w.apply(ev({ op: "set_level", level: { owner: "x", metric: "disk", value: 5, max: 10 } }));
    w.apply(ev({ op: "remove_place", id: "x" }));
    expect(w.places.has("x")).toBe(false);
    expect(w.level("x", "disk")).toBeUndefined();
    w.apply(ev({ op: "set_state", id: "j1", state: "running" }));
    expect(w.entities.get("j1")?.state).toBe("running");
    expect(w.apply(ev({ op: "flash", at: "q", kind: "oom" }))).toEqual({ type: "flash", at: "q", kind: "oom" });
    w.apply(ev({ op: "stale", source: "gitlab", since_ms: 7 }));
    expect(w.stale.get("gitlab")).toBe(7);
    w.apply(ev({ op: "fresh", source: "gitlab" }));
    expect(w.stale.size).toBe(0);
  });

  test("a move of an unknown entity is harmless", () => {
    const w = ClientWorld.fromSnapshot(snap());
    expect(w.apply(ev({ op: "move", id: "ghost", to: "q" }))).toEqual({ type: "none" });
  });
});
```

- [ ] **Step 3: Запустить — тесты падают**

Run: `cd web && npm test`
Expected: FAIL — `Failed to resolve import "./world"`.

- [ ] **Step 4: Реализовать зеркало мира и клиент API**

`web/src/world.ts`:

```ts
import type { Entity, Id, Level, Place, Snapshot, WorldEvent } from "./types";

/** What an event means for animation. */
export type Effect =
  | { type: "appear"; id: Id; at: Id }
  | { type: "move"; id: Id; from: Id; to: Id }
  | { type: "gone"; id: Id; from: Id }
  | { type: "flash"; at: Id; kind: string }
  | { type: "none" };

const NONE: Effect = { type: "none" };
const levelKey = (owner: Id, metric: string) => `${owner}\u001f${metric}`;

/** The browser's copy of a scene's world. The server sends only valid events. */
export class ClientWorld {
  seq = 0;
  places = new Map<Id, Place>();
  entities = new Map<Id, Entity>();
  levels = new Map<string, Level>();
  stale = new Map<string, number>();

  static fromSnapshot(s: Snapshot): ClientWorld {
    const w = new ClientWorld();
    w.seq = s.seq;
    for (const p of s.places) w.places.set(p.id, p);
    for (const e of s.entities) w.entities.set(e.id, e);
    for (const l of s.levels) w.levels.set(levelKey(l.owner, l.metric), l);
    for (const [source, since] of Object.entries(s.stale)) w.stale.set(source, since);
    return w;
  }

  level(owner: Id, metric: string): Level | undefined {
    return this.levels.get(levelKey(owner, metric));
  }

  private dropLevelsOf(owner: Id): void {
    for (const [key, l] of this.levels) if (l.owner === owner) this.levels.delete(key);
  }

  apply(ev: WorldEvent): Effect {
    this.seq = ev.seq;
    const c = ev.change;
    switch (c.op) {
      case "upsert_place":
        this.places.set(c.place.id, c.place);
        return NONE;
      case "remove_place":
        this.places.delete(c.id);
        this.dropLevelsOf(c.id);
        return NONE;
      case "appear":
        this.entities.set(c.entity.id, c.entity);
        return { type: "appear", id: c.entity.id, at: c.entity.place };
      case "move": {
        const e = this.entities.get(c.id);
        if (!e) return NONE;
        this.entities.set(c.id, { ...e, place: c.to, since_ms: ev.ts_ms });
        return { type: "move", id: c.id, from: e.place, to: c.to };
      }
      case "set_state": {
        const e = this.entities.get(c.id);
        if (e) this.entities.set(c.id, { ...e, state: c.state });
        return NONE;
      }
      case "set_level":
        this.levels.set(levelKey(c.level.owner, c.level.metric), c.level);
        return NONE;
      case "clear_level":
        this.levels.delete(levelKey(c.owner, c.metric));
        return NONE;
      case "flash":
        return { type: "flash", at: c.at, kind: c.kind };
      case "gone": {
        const e = this.entities.get(c.id);
        if (!e) return NONE;
        this.entities.delete(c.id);
        this.dropLevelsOf(c.id);
        return { type: "gone", id: c.id, from: e.place };
      }
      case "stale":
        this.stale.set(c.source, c.since_ms);
        return NONE;
      case "fresh":
        this.stale.delete(c.source);
        return NONE;
    }
  }
}
```

`web/src/api.ts`:

```ts
import { ClientWorld, type Effect } from "./world";
import type { SceneInfo, Snapshot, WorldEvent } from "./types";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  return (await res.json()) as T;
}

export const fetchScenes = () => getJson<SceneInfo[]>("/api/v1/scenes");
export const fetchSnapshot = (id: string) => getJson<Snapshot>(`/api/v1/scenes/${encodeURIComponent(id)}/snapshot`);

export interface Subscription {
  close(): void;
}

/**
 * Keeps a world in sync: snapshot, then events after its seq. Any break —
 * network, server restart, 410 for a gap — means a fresh snapshot.
 */
export function follow(
  sceneId: string,
  onWorld: (world: ClientWorld, effects: Effect[]) => void,
  onStatus: (status: "live" | "reconnecting") => void,
): Subscription {
  let closed = false;
  let source: EventSource | null = null;
  let streamDone: (() => void) | null = null;

  const stream = (world: ClientWorld) =>
    new Promise<void>((done) => {
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
        const world = ClientWorld.fromSnapshot(await fetchSnapshot(sceneId));
        onWorld(world, []);
        await stream(world);
      } catch {
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

- [ ] **Step 5: Запустить — тесты и сборка проходят**

Run: `cd web && npm test && npm run build`
Expected: `5 passed`; `tsc` без ошибок; `dist/index.html` создан.

- [ ] **Step 6: Commit**

```bash
git add web/package.json web/package-lock.json web/tsconfig.json web/vite.config.ts web/index.html web/src/
git commit -m "feat(web): scaffold, world mirror and snapshot+SSE client

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 11: Веб — раскладка сцены CI и цвета

**Files:**
- Create: `web/src/colors.ts`, `web/src/colors.test.ts`, `web/src/scenes/ci/layout.ts`, `web/src/scenes/ci/layout.test.ts`

**Interfaces:**
- Consumes: `ClientWorld` (задача 10); id и виды мест из задачи 6.
- Produces:
  - `colors.ts`: `PALETTE` (8 цветов), `NEUTRAL`, `class ColorBook { get(key): string }` — первые 8 ключей получают цвета палитры по порядку появления, остальные и `""`/`"~other"` — `NEUTRAL`.
  - `layout.ts`: `WIDTH = 1200`, `MAX_TOKENS_IN_ZONE = 40`, `layoutCi(world): Layout`, где `Layout { width, height, boxes: Map<Id, PlaceBox>, tokens: Map<Id, Spot>, overflow: Map<Id, number>, walls: Wall[], titles: {x,y,text}[] }`, `PlaceBox = Rect & { id, kind, label }`. Сущности в `sink` и сверх лимита зоны — без `Spot`.

- [ ] **Step 1: Написать падающие тесты**

`web/src/colors.test.ts`:

```ts
import { expect, test } from "vitest";
import { ColorBook, NEUTRAL, PALETTE } from "./colors";

test("a key keeps its color; the ninth and 'other' are neutral", () => {
  const book = new ColorBook();
  const keys = ["a", "b", "c", "d", "e", "f", "g", "h"];
  expect(keys.map((k) => book.get(k))).toEqual(PALETTE);
  expect(book.get("a")).toBe(PALETTE[0]);
  expect(book.get("i")).toBe(NEUTRAL);
  expect(book.get("~other")).toBe(NEUTRAL);
  expect(book.get("")).toBe(NEUTRAL);
});
```

`web/src/scenes/ci/layout.test.ts`:

```ts
import { describe, expect, test } from "vitest";
import { ClientWorld } from "../../world";
import type { Entity, Level, Place } from "../../types";
import { layoutCi, MAX_TOKENS_IN_ZONE, type PlaceBox, type Spot } from "./layout";

type P = Partial<Place> & { id: string };
type E = Partial<Entity> & { id: string; place: string };

function world(places: P[], entities: E[] = [], levels: Level[] = []): ClientWorld {
  return ClientWorld.fromSnapshot({
    seq: 1,
    stale: {},
    levels,
    places: places.map((p) => ({ kind: "zone", parent: null, label: p.id, capacity: null, attrs: {}, ...p })),
    entities: entities.map((e, i) => ({ kind: "job", state: "pending", attrs: {}, since_ms: i, ...e })),
  });
}

const zones: P[] = [
  { id: "projects" }, { id: "queue", kind: "queue" }, { id: "stuck", kind: "stuck" },
  { id: "hosts" }, { id: "nohost" }, { id: "exits" }, { id: "sink", kind: "sink" },
];

const host: P[] = [
  { id: "host:box", kind: "host", parent: "hosts" },
  { id: "process:a/p", kind: "process", parent: "host:box", label: "p", capacity: 2, attrs: { agent: "a", online: true } },
  { id: "slot:a/p/0", kind: "slot", parent: "process:a/p", label: "#0" },
  { id: "slot:a/p/1", kind: "slot", parent: "process:a/p", label: "#1" },
  { id: "process:b/q", kind: "process", parent: "host:box", label: "q", capacity: 1, attrs: { agent: "b", online: true } },
  { id: "slot:b/q/0", kind: "slot", parent: "process:b/q", label: "#0" },
];

const inside = (s: Spot, b: PlaceBox) => s.x >= b.x && s.x <= b.x + b.w && s.y >= b.y && s.y <= b.y + b.h;

describe("layoutCi", () => {
  test("every token sits inside its place", () => {
    const w = world(
      [...zones, ...host,
        { id: "project:g/a", kind: "project", parent: "projects", label: "a" },
        { id: "exit:prod", kind: "exit", parent: "exits", label: "prod" },
        { id: "runner:7", kind: "process", parent: "nohost", label: "runner7" }],
      [
        { id: "job:1", place: "project:g/a" }, { id: "job:2", place: "queue" }, { id: "job:3", place: "stuck" },
        { id: "job:4", place: "slot:a/p/1" }, { id: "job:5", place: "process:a/p" }, { id: "job:6", place: "exit:prod" },
        { id: "job:7", place: "runner:7" },
      ],
    );
    const l = layoutCi(w);
    expect(l.tokens.size).toBe(7);
    for (const e of w.entities.values()) {
      const spot = l.tokens.get(e.id)!;
      const box = l.boxes.get(e.place)!;
      expect(inside(spot, box), `${e.id} in ${e.place}`).toBe(true);
    }
  });

  test("a huge queue shows 40 tokens and counts the rest", () => {
    const jobs: E[] = Array.from({ length: 200 }, (_, i) => ({ id: `job:${i}`, place: "queue" }));
    const l = layoutCi(world(zones, jobs));
    const queue = l.boxes.get("queue")!;
    const shown = jobs.filter((j) => l.tokens.has(j.id));
    expect(shown.length).toBe(MAX_TOKENS_IN_ZONE);
    expect(l.overflow.get("queue")).toBe(160);
    for (const j of shown) expect(inside(l.tokens.get(j.id)!, queue)).toBe(true);
    expect(l.tokens.has("job:0")).toBe(true);
    expect(l.tokens.has("job:199")).toBe(false);
  });

  test("processes of different agents are split by a wall", () => {
    const l = layoutCi(world([...zones, ...host]));
    expect(l.walls.length).toBe(1);
    const p = l.boxes.get("process:a/p")!;
    const q = l.boxes.get("process:b/q")!;
    expect(q.y).toBeGreaterThan(p.y + p.h);
  });

  test("the sink has no spots and the layout is deterministic", () => {
    const w = world([...zones, ...host], [{ id: "job:1", place: "sink" }]);
    const a = layoutCi(w);
    expect(a.tokens.has("job:1")).toBe(false);
    expect(JSON.stringify([...a.boxes])).toBe(JSON.stringify([...layoutCi(w).boxes]));
  });

  test("the scene grows with hosts", () => {
    const one = layoutCi(world([...zones, ...host])).height;
    const two = layoutCi(world([...zones, ...host,
      { id: "host:other", kind: "host", parent: "hosts" },
      { id: "process:c/r", kind: "process", parent: "host:other", label: "r", attrs: { agent: "c" } },
      { id: "slot:c/r/0", kind: "slot", parent: "process:c/r", label: "#0" }])).height;
    expect(two).toBeGreaterThan(one);
  });
});
```

- [ ] **Step 2: Запустить — тесты падают**

Run: `cd web && npm test`
Expected: FAIL — `Failed to resolve import "./colors"` и `"./layout"`.

- [ ] **Step 3: Реализовать**

`web/src/colors.ts`:

```ts
/** Categorical palette, fixed order. A color follows its key, never its rank. */
export const PALETTE = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948"];
export const NEUTRAL = "#888780";

export class ColorBook {
  private assigned = new Map<string, string>();

  get(key: string): string {
    if (!key || key === "~other") return NEUTRAL;
    const known = this.assigned.get(key);
    if (known) return known;
    if (this.assigned.size >= PALETTE.length) return NEUTRAL;
    const color = PALETTE[this.assigned.size];
    this.assigned.set(key, color);
    return color;
  }
}
```

`web/src/scenes/ci/layout.ts`:

```ts
import type { ClientWorld } from "../../world";
import type { Entity, Id, Place } from "../../types";

export interface Rect { x: number; y: number; w: number; h: number }
export interface PlaceBox extends Rect { id: Id; kind: string; label: string }
export interface Spot { x: number; y: number }
export interface Wall { x1: number; y1: number; x2: number; y2: number }
export interface Layout {
  width: number;
  height: number;
  boxes: Map<Id, PlaceBox>;
  tokens: Map<Id, Spot>;
  overflow: Map<Id, number>;
  walls: Wall[];
  titles: { x: number; y: number; text: string }[];
}

export const WIDTH = 1200;
export const MAX_TOKENS_IN_ZONE = 40;

const TOP = 48;
const COL = { projects: { x: 20, w: 200 }, queue: { x: 270, w: 220 }, hosts: { x: 540, w: 420 }, exits: { x: 1010, w: 170 } };
const CELL = 26, PER_ROW = 7, ZONE_HEAD = 28, ZONE_PAD = 12;
const SLOT_H = 34, SLOT_GAP = 6, PROC_HEAD = 24, HOST_HEAD = 30, DISK_H = 26;
const MAX_IN_ROW = 10;

const byLabel = (a: Place, b: Place) => a.label.localeCompare(b.label, undefined, { numeric: true }) || a.id.localeCompare(b.id);
const agentOf = (p: Place) => String(p.attrs.agent ?? "");

function children(w: ClientWorld, parent: Id, kind: string): Place[] {
  return [...w.places.values()].filter((p) => p.parent === parent && p.kind === kind).sort(byLabel);
}

function residents(w: ClientWorld, place: Id): Entity[] {
  return [...w.entities.values()].filter((e) => e.place === place).sort((a, b) => a.since_ms - b.since_ms || a.id.localeCompare(b.id));
}

function box(out: Layout, p: Place | { id: Id; kind: string; label: string }, r: Rect): void {
  out.boxes.set(p.id, { id: p.id, kind: p.kind, label: p.label, ...r });
}

/** A grid zone (queue, stuck): tokens in rows, at most MAX_TOKENS_IN_ZONE. Returns the bottom. */
function gridZone(w: ClientWorld, out: Layout, place: Place | undefined, x: number, y: number, width: number, minH: number): number {
  if (!place) return y;
  const people = residents(w, place.id);
  const shown = people.slice(0, MAX_TOKENS_IN_ZONE);
  const rows = Math.max(1, Math.ceil(shown.length / PER_ROW));
  const h = Math.max(minH, ZONE_HEAD + rows * CELL + ZONE_PAD);
  box(out, place, { x, y, w: width, h });
  shown.forEach((e, i) =>
    out.tokens.set(e.id, { x: x + ZONE_PAD + CELL / 2 + (i % PER_ROW) * CELL, y: y + ZONE_HEAD + CELL / 2 + Math.floor(i / PER_ROW) * CELL }),
  );
  if (people.length > shown.length) out.overflow.set(place.id, people.length - shown.length);
  return y + h;
}

/** Tokens in a row starting at x, at most MAX_IN_ROW. */
function row(w: ClientWorld, out: Layout, place: Id, x: number, y: number, step: number): void {
  residents(w, place).slice(0, MAX_IN_ROW).forEach((e, i) => out.tokens.set(e.id, { x: x + i * step, y }));
}

export function layoutCi(w: ClientWorld): Layout {
  const out: Layout = { width: WIDTH, height: 0, boxes: new Map(), tokens: new Map(), overflow: new Map(), walls: [], titles: [] };
  const bottoms: number[] = [];
  const title = (id: Id, x: number) => out.titles.push({ x, y: TOP - 14, text: w.places.get(id)?.label ?? "" });

  // Projects: where jobs are born.
  title("projects", COL.projects.x);
  let y = TOP;
  for (const p of children(w, "projects", "project")) {
    box(out, p, { x: COL.projects.x, y, w: COL.projects.w, h: 36 });
    row(w, out, p.id, COL.projects.x + COL.projects.w - 16, y + 18, -12);
    y += 46;
  }
  bottoms.push(y);

  // Queue and the stuck zone under it.
  let qy = gridZone(w, out, w.places.get("queue"), COL.queue.x, TOP, COL.queue.w, 110);
  qy = gridZone(w, out, w.places.get("stuck"), COL.queue.x, qy + 20, COL.queue.w, 80);
  bottoms.push(qy);

  // Hosts → processes → slots; a wall between processes of different agents.
  title("hosts", COL.hosts.x);
  let hy = TOP;
  for (const host of children(w, "hosts", "host")) {
    const top = hy;
    let cy = hy + HOST_HEAD;
    const procs = children(w, host.id, "process").sort((a, b) => agentOf(a).localeCompare(agentOf(b)) || byLabel(a, b));
    procs.forEach((proc, i) => {
      if (i > 0 && agentOf(procs[i - 1]) !== agentOf(proc)) {
        out.walls.push({ x1: COL.hosts.x + 10, y1: cy + 2, x2: COL.hosts.x + COL.hosts.w - 10, y2: cy + 2 });
        cy += 12;
      }
      const slots = children(w, proc.id, "slot");
      const ph = PROC_HEAD + slots.length * (SLOT_H + SLOT_GAP) + 6;
      box(out, proc, { x: COL.hosts.x + 10, y: cy, w: COL.hosts.w - 20, h: ph });
      row(w, out, proc.id, COL.hosts.x + 170, cy + 12, 16);
      slots.forEach((s, si) => {
        const sy = cy + PROC_HEAD + si * (SLOT_H + SLOT_GAP);
        box(out, s, { x: COL.hosts.x + 20, y: sy, w: COL.hosts.w - 40, h: SLOT_H });
        row(w, out, s.id, COL.hosts.x + 38, sy + SLOT_H / 2, 14);
      });
      cy += ph + 8;
    });
    if (w.level(host.id, "disk")) cy += DISK_H;
    box(out, host, { x: COL.hosts.x, y: top, w: COL.hosts.w, h: cy - top + 6 });
    hy = cy + 26;
  }

  // Runners we see in GitLab but have no agent for.
  const loose = children(w, "nohost", "process");
  if (loose.length > 0) {
    const top = hy;
    let ly = hy + HOST_HEAD;
    for (const r of loose) {
      box(out, r, { x: COL.hosts.x + 10, y: ly, w: COL.hosts.w - 20, h: 30 });
      row(w, out, r.id, COL.hosts.x + 170, ly + 15, 16);
      ly += 38;
    }
    box(out, { id: "nohost", kind: "zone", label: w.places.get("nohost")?.label ?? "" }, { x: COL.hosts.x, y: top, w: COL.hosts.w, h: ly - top });
    hy = ly + 20;
  }
  bottoms.push(hy);

  // Exits.
  title("exits", COL.exits.x);
  let ey = TOP;
  for (const x of children(w, "exits", "exit")) {
    box(out, x, { x: COL.exits.x, y: ey, w: COL.exits.w, h: 44 });
    row(w, out, x.id, COL.exits.x + 20, ey + 22, 16);
    ey += 56;
  }
  bottoms.push(ey);

  out.height = Math.max(...bottoms) + 24;
  return out;
}
```

- [ ] **Step 4: Запустить — тесты проходят**

Run: `cd web && npm test && npm run build`
Expected: `11 passed` (5 + 1 + 5), сборка без ошибок.

- [ ] **Step 5: Commit**

```bash
git add web/src/
git commit -m "feat(web): CI scene layout and stable project colors

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 12: Веб — отрисовка и приложение

**Files:**
- Create: `web/src/i18n.ts`, `web/src/index.css`, `web/src/scenes/ci/renderer.ts`, `web/src/scenes/ci/CiView.tsx`, `web/src/App.tsx`
- Modify: `web/src/main.tsx` — заменить целиком

**Interfaces:**
- Consumes: `ClientWorld`, `Effect`, `follow`, `fetchScenes`, `layoutCi`, `ColorBook`.
- Produces: `CiRenderer(svg) { update(world, effects); destroy() }` — перелёт токенов 700 мс, затухание 700 мс, вспышка OOM 1.2 с; `<CiView onStatus onStale />`; `<App />` с вкладками сцен, статусом соединения и баннером «данные устарели».

Рендерер не покрывается юнит-тестами (это рисование); его проверка — ручная, по шагу 3, на живом демо. Вся логика «где что стоит» уже покрыта тестами раскладки.

- [ ] **Step 1: Написать код интерфейса**

`web/src/i18n.ts`:

```ts
// Every UI string lives here.
export const t = {
  connecting: "connecting…",
  live: "live",
  reconnecting: "reconnecting…",
  offline: "server unreachable",
  stale: (source: string, seconds: number) => `${source} data is ${seconds}s old — showing the last known state`,
  free: "free",
  running: "running",
  disk: "disk",
  agentOffline: "agent offline",
  slots: (n: number) => `${n} slot${n === 1 ? "" : "s"}`,
};
```

`web/src/index.css`:

```css
:root {
  --bg: #fbfbfa;
  --surface: #ffffff;
  --surface-2: #f3f2ee;
  --text: #1a1a19;
  --muted: #6b6a66;
  --border: #dedcd5;
  --ok: #1baf7a;
  --warn: #eda100;
  --bad: #e34948;
  color-scheme: light dark;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #141413;
    --surface: #1d1d1b;
    --surface-2: #262624;
    --text: #f0efec;
    --muted: #9c9a92;
    --border: #3a3936;
  }
}
body { margin: 0; background: var(--bg); color: var(--text); font: 14px/1.4 system-ui, sans-serif; }
header { display: flex; align-items: center; gap: 16px; padding: 10px 20px; border-bottom: 1px solid var(--border); }
header h1 { font-size: 16px; margin: 0; }
.tabs button { background: none; border: 1px solid var(--border); border-radius: 6px; padding: 4px 10px; color: var(--text); cursor: pointer; }
.tabs button.on { background: var(--surface-2); }
.status { margin-left: auto; color: var(--muted); font-size: 13px; }
.banner { background: color-mix(in srgb, var(--warn) 18%, transparent); padding: 8px 20px; font-size: 13px; }
svg.scene { display: block; width: 100%; height: auto; }
.place > rect:first-child { fill: var(--surface); stroke: var(--border); }
.place.zone > rect:first-child, .place.host > rect:first-child { fill: var(--surface-2); }
.place.stuck > rect:first-child { fill: color-mix(in srgb, var(--bad) 8%, var(--surface)); stroke: var(--bad); stroke-dasharray: 5 4; }
.place.slot > rect:first-child { fill: var(--bg); }
.place .fill { opacity: 0.35; stroke: none; }
.place .fill.oom { fill: var(--bad) !important; opacity: 0.6; }
.place.flash > rect:first-child { fill: color-mix(in srgb, var(--bad) 30%, var(--surface)); stroke: var(--bad); }
.place.offline { opacity: 0.5; }
.label { fill: var(--text); font-size: 12px; font-weight: 600; }
.extra { fill: var(--muted); font-size: 11px; text-anchor: end; }
.title { fill: var(--muted); font-size: 12px; }
.wall { stroke: var(--muted); stroke-width: 2; }
.token { stroke: var(--surface); stroke-width: 2; }
.token.waiting_limit { stroke: var(--warn); stroke-width: 3; }
.token.oom, .token.failed { stroke: var(--bad); stroke-width: 3; }
```

`web/src/scenes/ci/renderer.ts`:

```ts
import type { ClientWorld, Effect } from "../../world";
import type { Id } from "../../types";
import { ColorBook } from "../../colors";
import { t } from "../../i18n";
import { layoutCi, type Layout, type PlaceBox } from "./layout";

const NS = "http://www.w3.org/2000/svg";
const MOVE_MS = 700;
const FADE_MS = 700;
const FLASH_MS = 1200;
const RADIUS = 7;
const DEPTH: Record<string, number> = { zone: 0, queue: 0, stuck: 0, project: 1, host: 1, exit: 1, process: 2, slot: 3 };

interface Token {
  el: SVGCircleElement;
  x: number; y: number;
  fromX: number; fromY: number;
  toX: number; toY: number;
  start: number;
  fadeStart: number | null;
  hidden: boolean;
}

interface PlaceEls { g: SVGGElement; rect: SVGRectElement; fill: SVGRectElement; label: SVGTextElement; extra: SVGTextElement }

function el<K extends keyof SVGElementTagNameMap>(tag: K, parent: Element, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const node = document.createElementNS(NS, tag);
  set(node, attrs);
  parent.appendChild(node);
  return node;
}

function set(node: Element, attrs: Record<string, string | number>): void {
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
}

const ease = (p: number) => 1 - Math.pow(1 - p, 3);
const gib = (bytes: number) => (bytes / 2 ** 30).toFixed(1);

/** Draws a CI world into an SVG and animates tokens between places. */
export class CiRenderer {
  private placesLayer: SVGGElement;
  private overlay: SVGGElement;
  private tokenLayer: SVGGElement;
  private placeEls = new Map<Id, PlaceEls>();
  private tokens = new Map<Id, Token>();
  private births = new Map<Id, Id>();
  private flashes = new Map<Id, number>();
  private colors = new ColorBook();
  private world: ClientWorld | null = null;
  private dirty = false;
  private raf = 0;

  constructor(private svg: SVGSVGElement) {
    this.placesLayer = el("g", svg);
    this.overlay = el("g", svg);
    this.tokenLayer = el("g", svg);
    this.raf = requestAnimationFrame(this.frame);
  }

  update(world: ClientWorld, effects: Effect[]): void {
    this.world = world;
    const now = performance.now();
    for (const e of effects) {
      if (e.type === "flash") this.flashes.set(e.at, now + FLASH_MS);
      if (e.type === "appear") this.births.set(e.id, e.at);
    }
    this.dirty = true;
  }

  destroy(): void {
    cancelAnimationFrame(this.raf);
    this.svg.replaceChildren();
  }

  private frame = (now: number): void => {
    if (this.dirty && this.world) {
      this.dirty = false;
      const layout = layoutCi(this.world);
      this.svg.setAttribute("viewBox", `0 0 ${layout.width} ${layout.height}`);
      this.drawPlaces(this.world, layout, now);
      this.drawOverlay(layout);
      this.placeTokens(this.world, layout, now);
    }
    this.animate(now);
    this.raf = requestAnimationFrame(this.frame);
  };

  private drawPlaces(w: ClientWorld, layout: Layout, now: number): void {
    for (const [id, p] of this.placeEls) {
      if (!layout.boxes.has(id)) {
        p.g.remove();
        this.placeEls.delete(id);
      }
    }
    const boxes = [...layout.boxes.values()].sort((a, b) => (DEPTH[a.kind] ?? 0) - (DEPTH[b.kind] ?? 0));
    for (const b of boxes) {
      let p = this.placeEls.get(b.id);
      if (!p) {
        const g = el("g", this.placesLayer, { class: `place ${b.kind}` });
        p = { g, rect: el("rect", g, { rx: 6 }), fill: el("rect", g, { rx: 6, class: "fill" }), label: el("text", g, { class: "label" }), extra: el("text", g, { class: "extra" }) };
        this.placeEls.set(b.id, p);
      }
      this.placesLayer.appendChild(p.g); // keeps depth order
      set(p.rect, { x: b.x, y: b.y, width: b.w, height: b.h });
      p.label.textContent = b.kind === "slot" ? "" : b.label;
      set(p.label, { x: b.x + 10, y: b.y + 18 });
      p.g.classList.toggle("flash", (this.flashes.get(b.id) ?? 0) > now);
      p.g.classList.toggle("offline", w.places.get(b.id)?.attrs.online === false);
      this.drawFill(w, b, p);
    }
  }

  private drawFill(w: ClientWorld, b: PlaceBox, p: PlaceEls): void {
    set(p.fill, { x: b.x, y: b.y, width: 0, height: b.h, class: "fill" });
    p.fill.style.fill = "";
    p.extra.textContent = "";
    if (b.kind === "slot") {
      set(p.extra, { x: b.x + b.w - 10, y: b.y + 21 });
      const job = [...w.entities.values()].find((e) => e.place === b.id);
      if (!job) {
        p.extra.textContent = t.free;
        return;
      }
      const mem = w.level(job.id, "mem");
      const frac = mem?.max ? Math.min(1, mem.value / mem.max) : 0;
      set(p.fill, { width: b.w * frac });
      p.fill.style.fill = frac >= 0.9 ? "var(--warn)" : this.colors.get(String(job.attrs.project ?? ""));
      p.fill.classList.toggle("oom", job.state === "oom");
      const usage = mem?.max ? `${gib(mem.value)} / ${gib(mem.max)} GB` : t.running;
      p.extra.textContent = `${String(job.attrs.name ?? "")} · ${usage}`;
    } else if (b.kind === "host") {
      const disk = w.level(b.id, "disk");
      if (!disk?.max) return;
      const frac = Math.min(1, disk.value / disk.max);
      set(p.fill, { x: b.x + 10, y: b.y + b.h - 22, width: (b.w - 20) * frac, height: 8 });
      p.fill.style.fill = frac >= 0.85 ? "var(--bad)" : frac >= 0.75 ? "var(--warn)" : "var(--ok)";
      p.extra.textContent = `${t.disk} ${Math.round(frac * 100)}%`;
      set(p.extra, { x: b.x + b.w - 10, y: b.y + b.h - 28 });
    } else if (b.kind === "process") {
      const place = w.places.get(b.id);
      p.extra.textContent = place?.attrs.online === false ? t.agentOffline : place?.capacity ? t.slots(place.capacity) : "";
      set(p.extra, { x: b.x + b.w - 10, y: b.y + 16 });
    }
  }

  private drawOverlay(layout: Layout): void {
    this.overlay.replaceChildren();
    for (const wl of layout.walls) el("line", this.overlay, { ...wl, class: "wall" });
    for (const ti of layout.titles) el("text", this.overlay, { x: ti.x, y: ti.y, class: "title" }).textContent = ti.text;
    for (const [id, n] of layout.overflow) {
      const b = layout.boxes.get(id);
      if (b) el("text", this.overlay, { x: b.x + b.w - 10, y: b.y + 18, class: "extra" }).textContent = `+${n}`;
    }
  }

  private placeTokens(w: ClientWorld, layout: Layout, now: number): void {
    for (const [id, tk] of this.tokens) if (!w.entities.has(id) && tk.fadeStart === null) tk.fadeStart = now;
    for (const e of w.entities.values()) {
      const spot = layout.tokens.get(e.id);
      let tk = this.tokens.get(e.id);
      if (!tk) {
        if (!spot) continue;
        const born = this.births.get(e.id);
        const from = born ? layout.boxes.get(born) : undefined;
        const sx = from ? from.x + from.w - 16 : spot.x;
        const sy = from ? from.y + 18 : spot.y;
        this.births.delete(e.id);
        tk = { el: el("circle", this.tokenLayer, { r: RADIUS }), x: sx, y: sy, fromX: sx, fromY: sy, toX: spot.x, toY: spot.y, start: now, fadeStart: null, hidden: false };
        this.tokens.set(e.id, tk);
      }
      tk.el.style.fill = this.colors.get(String(e.attrs.project ?? ""));
      tk.el.setAttribute("class", `token ${e.state}`);
      if (e.place === "sink") {
        if (tk.fadeStart === null) tk.fadeStart = now;
        continue;
      }
      tk.hidden = !spot;
      if (spot && (spot.x !== tk.toX || spot.y !== tk.toY)) {
        tk.fromX = tk.x;
        tk.fromY = tk.y;
        tk.toX = spot.x;
        tk.toY = spot.y;
        tk.start = now;
      }
    }
  }

  private animate(now: number): void {
    for (const [id, tk] of this.tokens) {
      const p = Math.min(1, (now - tk.start) / MOVE_MS);
      tk.x = tk.fromX + (tk.toX - tk.fromX) * ease(p);
      tk.y = tk.fromY + (tk.toY - tk.fromY) * ease(p);
      set(tk.el, { cx: tk.x, cy: tk.y });
      let opacity = tk.hidden ? 0 : 1;
      if (tk.fadeStart !== null) {
        opacity = Math.max(0, 1 - (now - tk.fadeStart) / FADE_MS);
        if (opacity === 0) {
          tk.el.remove();
          this.tokens.delete(id);
          continue;
        }
      }
      tk.el.style.opacity = String(opacity);
    }
    for (const [id, until] of this.flashes) {
      if (until <= now) {
        this.flashes.delete(id);
        this.dirty = true;
      }
    }
  }
}
```

`web/src/scenes/ci/CiView.tsx`:

```tsx
import { useEffect, useRef } from "react";
import { follow } from "../../api";
import { CiRenderer } from "./renderer";

interface Props {
  onStatus: (status: "live" | "reconnecting") => void;
  onStale: (stale: Record<string, number>) => void;
}

export function CiView({ onStatus, onStale }: Props) {
  const ref = useRef<SVGSVGElement>(null);
  useEffect(() => {
    const svg = ref.current;
    if (!svg) return;
    const renderer = new CiRenderer(svg);
    let lastStale = "";
    const sub = follow(
      "ci",
      (world, effects) => {
        renderer.update(world, effects);
        const stale = JSON.stringify([...world.stale]);
        if (stale !== lastStale) {
          lastStale = stale;
          onStale(Object.fromEntries(world.stale));
        }
      },
      onStatus,
    );
    return () => {
      sub.close();
      renderer.destroy();
    };
  }, [onStatus, onStale]);
  return <svg ref={ref} className="scene" role="img" aria-label="CI scene" />;
}
```

`web/src/App.tsx`:

```tsx
import { useEffect, useState } from "react";
import { fetchScenes } from "./api";
import { t } from "./i18n";
import type { SceneInfo } from "./types";
import { CiView } from "./scenes/ci/CiView";

type Status = "connecting" | "live" | "reconnecting" | "offline";

export function App() {
  const [scenes, setScenes] = useState<SceneInfo[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>("connecting");
  const [stale, setStale] = useState<Record<string, number>>({});
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    fetchScenes()
      .then((list) => {
        setScenes(list);
        setActive(list[0]?.id ?? null);
      })
      .catch(() => setStatus("offline"));
  }, []);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="app">
      <header>
        <h1>lorvaq</h1>
        <nav className="tabs">
          {scenes.map((s) => (
            <button key={s.id} className={s.id === active ? "on" : ""} onClick={() => setActive(s.id)}>
              {s.title}
            </button>
          ))}
        </nav>
        <span className="status">{t[status]}</span>
      </header>
      {Object.entries(stale).map(([source, since]) => (
        <div key={source} className="banner">
          {t.stale(source, Math.max(0, Math.round((now - since) / 1000)))}
        </div>
      ))}
      {active === "ci" && <CiView onStatus={setStatus} onStale={setStale} />}
    </div>
  );
}
```

Заменить `web/src/main.tsx` целиком:

```tsx
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./index.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
```

- [ ] **Step 2: Тесты и сборка**

Run: `cd web && npm test && npm run build`
Expected: `11 passed`, `tsc` и `vite build` без ошибок.

- [ ] **Step 3: Проверить вживую**

В одном терминале: `cargo run -- server --demo --listen 127.0.0.1:8080`
В другом: `cd web && npm run dev`, открыть `http://localhost:5173`.

Проверить глазами и записать в отчёт задачи, что увидено:
- кружки появляются у проектов слева и перелетают в очередь, затем в слоты хоста;
- слоты заливаются памятью; заливка рядом с лимитом жёлтая;
- OOM: слот вспыхивает красным, кружок с красной обводкой гаснет;
- джобы `demo/legacy` с тегом `docker` копятся в красной зоне «no runner» и через ~40 с гаснут;
- джобы `mobile` с жёлтой обводкой ждут в очереди при свободных слотах (лимит heavy);
- сборки улетают в `registry`, деплои — в `prod`;
- между процессами `shared` и `deploy` — стена; внизу хоста — полоска диска;
- остановить сервер: статус `reconnecting…`; запустить снова: сцена восстанавливается сама, без перезагрузки страницы;
- тёмная тема ОС: всё читается.

Если что-то из списка не так — чинить в этой задаче.

- [ ] **Step 4: Commit**

```bash
git add web/src/
git commit -m "feat(web): animated CI scene renderer and app shell

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Задача 13: Образ, compose и README

**Files:**
- Create: `Dockerfile`, `.dockerignore`, `compose.demo.yml`, `README.md`

**Interfaces:**
- Consumes: всё. Образ запускает `lorvaq server --demo --listen 0.0.0.0:8080` по умолчанию.

- [ ] **Step 1: Написать файлы сборки**

`.dockerignore`:

```text
target
web/node_modules
web/dist
.git
docs
```

`Dockerfile`:

```dockerfile
# Web UI
FROM node:22-alpine AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# Static binary with the UI embedded
FROM rust:1.91-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# rust-toolchain.toml is left out on purpose: the image already has 1.91.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY --from=web /web/dist ./web/dist
RUN cargo build --release --locked

FROM scratch
COPY --from=build /src/target/release/lorvaq /lorvaq
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/lorvaq"]
CMD ["server", "--demo", "--listen", "0.0.0.0:8080"]
```

`compose.demo.yml`:

```yaml
# Try lorvaq on simulated data: docker compose -f compose.demo.yml up --build
services:
  lorvaq:
    build: .
    image: lorvaq:dev
    ports:
      - "8080:8080"
    command: ["server", "--demo", "--listen", "0.0.0.0:8080"]
```

`README.md`:

```markdown
# lorvaq

Live scenes for real-time monitoring. Where Grafana shows numbers over time,
lorvaq shows **things in places that move**: a CI job travels from the queue
into a runner slot, its memory fills the slot, an OOM flashes red, a build flies
off to the registry. You see not only *how much* but *why and where*.

Status: early. This release runs the CI scene on a built-in simulator; GitLab,
the runner agent, history and settings come next.

## Try it

    docker compose -f compose.demo.yml up --build

Open http://localhost:8080.

## Develop

    cargo run -- server --demo          # API and SSE on :8080
    cd web && npm ci && npm run dev     # UI on :5173, proxied to :8080

Tests: `cargo test` and `cd web && npm test`.

## How it works

Every scene is described by four things: **places** (queue, hosts, slots),
**entities** in them (jobs), **moves** between places and **levels** against
limits (memory, disk). Sources feed a scene reducer, the reducer emits changes,
valid changes become numbered events; the browser takes a snapshot and follows
the event stream (SSE).

Design: `docs/design/`.

## License

MIT
```

- [ ] **Step 2: Собрать и запустить образ**

```bash
docker compose -f compose.demo.yml up -d --build
sleep 5
curl -fsS localhost:8080/healthz
curl -fsS localhost:8080/api/v1/scenes/ci/snapshot | head -c 300; echo
curl -fsS localhost:8080/ | head -c 200; echo
docker image inspect lorvaq:dev --format '{{.Size}}'
```

Expected: `healthz` — `{"status":"ok",...}`; снимок с местами сцены; `/` отдаёт собранный `index.html` (не страницу `NOT_BUILT`); размер образа — десятки мегабайт, не сотни.

Открыть `http://localhost:8080` и убедиться, что сцена живёт так же, как в задаче 12. Затем:

```bash
docker compose -f compose.demo.yml down
```

Expected: контейнер останавливается за секунды (SIGTERM обработан), а не через 10 с по таймауту.

- [ ] **Step 3: Финальная проверка всего репо**

```bash
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
(cd web && npm test && npm run build)
git grep -n -i -E -f ~/.config/lorvaq/denylist -- ':!web/src/scenes/ci/renderer.ts' || echo "clean"
```

Expected: всё зелёное; поиск по личному deny-листу (файл вне репо, spec §12) печатает `clean`.

- [ ] **Step 4: Commit**

```bash
git add Dockerfile .dockerignore compose.demo.yml README.md
git commit -m "feat: container image and demo compose

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
