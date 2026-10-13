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
        MIGRATOR
            .run(&pool)
            .await
            .context("database migration failed")?;
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
        let value: Option<Value> = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        value
            .map(|v| {
                serde_json::from_value(v)
                    .with_context(|| format!("setting {key:?} has an unexpected shape"))
            })
            .transpose()
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
        sqlx::query("DELETE FROM settings WHERE key = $1")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn admin_hash(&self) -> anyhow::Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT password_hash FROM admin WHERE id = 1")
                .fetch_optional(&self.pool)
                .await?,
        )
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

    /// A new admin password and every other session gone, or neither.
    pub async fn replace_admin_password(
        &self,
        hash: &str,
        keep_session_hash: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE admin SET password_hash = $1, updated_ms = $2 WHERE id = 1")
            .bind(hash)
            .bind(now_ms)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE token_hash <> $1")
            .bind(keep_session_hash)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stores the first admin; false when one already exists (two setups at once: one wins).
    pub async fn create_admin_if_absent(&self, hash: &str, now_ms: i64) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "INSERT INTO admin (id, password_hash, updated_ms) VALUES (1, $1, $2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(hash)
        .bind(now_ms)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Removes the admin and signs everybody out.
    pub async fn reset_admin(&self) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM admin").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM sessions")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_session(
        &self,
        token_hash: &str,
        now_ms: i64,
        ttl_ms: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO sessions (token_hash, created_ms, expires_ms) VALUES ($1, $2, $3)",
        )
        .bind(token_hash)
        .bind(now_ms)
        .bind(now_ms + ttl_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_valid(&self, token_hash: &str, now_ms: i64) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sessions WHERE token_hash = $1 AND expires_ms > $2)",
        )
        .bind(token_hash)
        .bind(now_ms)
        .fetch_one(&self.pool)
        .await?)
    }

    /// When the session expires, or None when there is no live session with this hash.
    pub async fn session_expiry(
        &self,
        token_hash: &str,
        now_ms: i64,
    ) -> anyhow::Result<Option<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT expires_ms FROM sessions WHERE token_hash = $1 AND expires_ms > $2",
        )
        .bind(token_hash)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_session(&self, token_hash: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_other_sessions(&self, keep_hash: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash <> $1")
            .bind(keep_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_sessions(&self, now_ms: i64) -> anyhow::Result<u64> {
        Ok(sqlx::query("DELETE FROM sessions WHERE expires_ms <= $1")
            .bind(now_ms)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    pub async fn scene_settings(&self, scene: &str) -> anyhow::Result<Option<Value>> {
        Ok(
            sqlx::query_scalar("SELECT value FROM scene_settings WHERE scene = $1")
                .bind(scene)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn put_scene_settings(
        &self,
        scene: &str,
        value: &Value,
        now_ms: i64,
    ) -> anyhow::Result<()> {
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
        let mut query =
            QueryBuilder::<Postgres>::new("INSERT INTO events (scene, seq, ts_ms, change) ");
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
        Ok(sqlx::query("DELETE FROM events WHERE ts_ms < $1")
            .bind(older_than_ms)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    pub async fn event_count(&self, scene: &str) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM events WHERE scene = $1")
                .bind(scene)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
