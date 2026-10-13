//! Wires scenes, sources, storage and HTTP into a running server.

use crate::auth::{self, Access};
use crate::cli::{AgentArgs, DbArgs, ServerArgs};
use crate::crypto::SecretBox;
use crate::history::{self, HistoryStatus};
use crate::http::{router, AppState, Live};
use crate::scene::{self, now_ms, SceneHandle};
use crate::scenes::ci::input::CiInput;
use crate::scenes::ci::settings::CiSettings;
use crate::scenes::ci::CiScene;
use crate::sources::gitlab::{self as gitlab_source, PollConfig};
use crate::store::Store;
use anyhow::Context;
use futures::FutureExt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// Event streams never end on their own; after a stop signal they get this long.
const STREAM_GRACE: Duration = Duration::from_secs(2);

/// The CI scene fed by the simulator. Needs a tokio runtime.
pub fn demo_scenes() -> anyhow::Result<Vec<SceneHandle>> {
    let ci = CiScene::new(crate::demo::settings())?;
    let (handle, tx) = scene::spawn(ci, Duration::from_secs(1));
    tokio::spawn(crate::demo::run(tx, 42));
    Ok(vec![handle])
}

async fn live_scenes(store: &Store) -> anyhow::Result<(Vec<SceneHandle>, mpsc::Sender<CiInput>)> {
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
    let (handle, inputs) = scene::spawn(ci, Duration::from_secs(1));
    Ok((vec![handle], inputs))
}

/// Everything the live server needs, from an open store.
pub async fn live_state(
    store: Store,
    secrets: SecretBox,
    public_url: Option<&str>,
) -> anyhow::Result<AppState> {
    let (scenes, ci_inputs) = live_scenes(&store).await?;
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
    let access = store
        .setting::<Access>(auth::ACCESS_KEY)
        .await?
        .unwrap_or_default();
    let cookie_secure = public_url.is_some_and(|u| u.starts_with("https://"));
    if !cookie_secure {
        tracing::warn!("LORVAQ_PUBLIC_URL is not an https URL, so the session cookie goes without the Secure flag; set it in production");
    }
    let http = crate::sources::gitlab::client::http_client();
    let agents = Arc::new(crate::hub::Hub::new(ci_inputs.clone()));
    let gitlab = gitlab_source::spawn(
        store.clone(),
        secrets.clone(),
        http.clone(),
        ci_inputs,
        PollConfig::default(),
    );
    let mut live = Live::new(store, secrets, cookie_secure, access, history);
    live.gitlab = Some(gitlab);
    live.agents = Some(agents);
    live.http = http;
    live.public_url = public_url
        .map(|u| u.trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    Ok(AppState {
        scenes,
        live: Some(live),
    })
}

pub async fn run_server(args: ServerArgs) -> anyhow::Result<()> {
    if args.demo {
        return serve(args.listen, AppState::demo(demo_scenes()?)).await;
    }
    let url = args
        .db
        .database_url
        .context("DATABASE_URL is required (or run with --demo)")?;
    let key = args
        .secret_key
        .context("LORVAQ_SECRET_KEY is required: create one with `lorvaq keygen`")?;
    let secrets = SecretBox::from_base64(&key)?;
    let store = Store::connect(&url).await?;
    let state = live_state(store, secrets, args.public_url.as_deref()).await?;
    serve(args.listen, state).await
}

/// `lorvaq agent`: runs until stopped.
pub async fn run_agent(args: AgentArgs) -> anyhow::Result<()> {
    use crate::agent::run::{connect_url, default_host_label, AgentConfig};
    connect_url(&args.server)?;
    let host = args.host_label.unwrap_or_else(default_host_label);
    let mut cfg = AgentConfig::new(&args.server, &args.join, &host);
    cfg.process = args.process;
    cfg.runner_metrics = args.runner_metrics;
    cfg.disk_path = args.disk_path;
    let docker = crate::agent::bollard_docker::BollardDocker::connect().await?;
    tracing::info!(host = %cfg.host_label, "lorvaq agent reports to {}", cfg.server);
    crate::agent::run::run(docker, cfg).await;
    Ok(())
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
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
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
