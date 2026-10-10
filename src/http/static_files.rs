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
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        return ([(header::CONTENT_TYPE, mime)], file.data).into_response();
    }
    // Client-side routes fall back to the app shell.
    match Assets::get("index.html") {
        Some(index) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            index.data,
        )
            .into_response(),
        None => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            NOT_BUILT,
        )
            .into_response(),
    }
}
