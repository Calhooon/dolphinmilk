//! Embedded UI asset serving via rust-embed.

#[cfg(feature = "embed-ui")]
mod embedded {
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "ui/dist"]
    pub struct UiAssets;
}

#[cfg(feature = "embed-ui")]
pub use embedded::UiAssets;

#[cfg(feature = "embed-ui")]
pub async fn serve_embedded_ui(uri: axum::http::Uri) -> impl axum::response::IntoResponse {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    // When nested under /ui, axum strips the prefix — we just need to remove the leading slash.
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() || path == "/" {
        "index.html"
    } else {
        path
    };

    match UiAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => {
            // SPA fallback: return index.html for unknown routes (hash routing)
            match UiAssets::get("index.html") {
                Some(content) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html".to_string())],
                    content.data.into_owned(),
                )
                    .into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
}
