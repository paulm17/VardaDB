use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "ui/dist/"]
struct UiAssets;

pub fn ui_router() -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/{*path}", get(static_handler))
}

async fn index_handler() -> Response {
    serve_asset("index.html")
}

async fn static_handler(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    serve_asset(&path)
}

fn serve_asset(path: &str) -> Response {
    match UiAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => {
            // SPA Fallback: If not found, serve index.html (unless it's an asset like .js/.css)
            // Simple heuristic: if it has an extension, return 404, else index.html
            if path.contains('.') {
                (StatusCode::NOT_FOUND, "Not Found").into_response()
            } else {
                // Try index.html for client-side routing
                match UiAssets::get("index.html") {
                    Some(content) => {
                        let mime = mime_guess::from_path("index.html").first_or_octet_stream();
                        (
                            [(header::CONTENT_TYPE, mime.as_ref())],
                            content.data.to_vec(),
                        )
                            .into_response()
                    }
                    None => (StatusCode::NOT_FOUND, "Index not found").into_response(),
                }
            }
        }
    }
}
