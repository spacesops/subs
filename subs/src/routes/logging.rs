use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::header,
    middleware::Next,
    response::Response,
};

const MAX_LOG_BODY_BYTES: usize = 1024 * 1024;
const MAX_LOG_TEXT_CHARS: usize = 4096;

fn is_api_path(path: &str) -> bool {
    !matches!(
        path,
        "/" | "/ui/operate" | "/ui/query" | "/ui/settings" | "/ui/transactions"
    ) && !path.starts_with("/ui/")
}

fn body_preview(content_type: Option<&str>, body: &[u8]) -> String {
    let body_text = match content_type {
        Some(ct) if ct.contains("json") || ct.starts_with("text/") => {
            match std::str::from_utf8(body) {
                Ok(s) => s.to_string(),
                Err(_) => format!("<non-utf8 body: {} bytes>", body.len()),
            }
        }
        Some(ct) if ct.contains("x-www-form-urlencoded") => match std::str::from_utf8(body) {
            Ok(s) => s.to_string(),
            Err(_) => format!("<non-utf8 form body: {} bytes>", body.len()),
        },
        _ => {
            if body.is_empty() {
                String::new()
            } else {
                format!(
                    "<binary body {} bytes, base64={}>",
                    body.len(),
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, body)
                )
            }
        }
    };

    if body_text.chars().count() > MAX_LOG_TEXT_CHARS {
        let clipped: String = body_text.chars().take(MAX_LOG_TEXT_CHARS).collect();
        format!("{clipped}... <truncated>")
    } else {
        body_text
    }
}

pub async fn log_api_requests(req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned);

    let bytes = match to_bytes(body, MAX_LOG_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                "api request {} {}{} body read failed: {}",
                method,
                path,
                query,
                e
            );
            let req = Request::from_parts(parts, Body::empty());
            return next.run(req).await;
        }
    };

    if is_api_path(&path) {
        if bytes.is_empty() {
            tracing::info!("api request {} {}{}", method, path, query);
        } else {
            let preview = body_preview(content_type.as_deref(), &bytes);
            tracing::info!(
                "api request {} {}{} payload={}",
                method,
                path,
                query,
                preview
            );
        }
    }

    let req = Request::from_parts(parts, Body::from(bytes));
    next.run(req).await
}
