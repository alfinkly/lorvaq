//! Wires scenes, sources and HTTP into a running server.

use crate::http::{router, AppState};
use crate::scene::{self, SceneHandle};
use crate::scenes::ci::CiScene;
use futures::FutureExt;
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
    let app = router(Arc::new(AppState::demo(scenes)));
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "lorvaq is listening");
    let signal = shutdown_signal().shared();
    let server = axum::serve(listener, app).with_graceful_shutdown(signal.clone());
    // Event streams never end on their own, so graceful shutdown would wait for
    // them forever (and docker would kill us after 10 s): give them a moment, then go.
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

const STREAM_GRACE: Duration = Duration::from_secs(2);

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
