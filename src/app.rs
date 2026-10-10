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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
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
