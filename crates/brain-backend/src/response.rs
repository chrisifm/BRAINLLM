//! Typed HTTP responses and safe negotiated defaults. No request values appear in error pages.
use axum::{
    Json,
    body::Body,
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Clone, Copy, Debug)]
pub struct DefaultError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseError {
    InvalidStatus,
    InvalidLocation,
}
impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidStatus => "Use a registered final HTTP status code",
            Self::InvalidLocation => "Location must be a safe same-site absolute path",
        })
    }
}
impl std::error::Error for ResponseError {}
impl IntoResponse for ResponseError {
    fn into_response(self) -> Response {
        let mut response = (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"schemaVersion":1,"error":{"code":"INVALID_HTTP_RESPONSE","fix":"Use a registered final status and safe same-site Location"}}))).into_response();
        response.extensions_mut().insert(DefaultError);
        response
    }
}
fn checked(code: u16) -> Result<StatusCode, ResponseError> {
    let status = StatusCode::from_u16(code).map_err(|_| ResponseError::InvalidStatus)?;
    if !(200..600).contains(&code) || status.canonical_reason().is_none() {
        return Err(ResponseError::InvalidStatus);
    }
    Ok(status)
}
fn location(path: &str) -> Result<HeaderValue, ResponseError> {
    let lower = path.to_ascii_lowercase();
    if path.len() > 8192
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('\\')
        || path.bytes().any(|b| b <= 32 || b == 127)
        || lower.starts_with("/%2f")
        || lower.contains("%5c")
        || lower.as_bytes().windows(3).any(|part| {
            part[0] == b'%'
                && (part[1] == b'0' || (part[1] == b'1') || part == b"%20" || part == b"%7f")
        })
    {
        return Err(ResponseError::InvalidLocation);
    }
    HeaderValue::from_str(path).map_err(|_| ResponseError::InvalidLocation)
}
/// Serialize a successful JSON response without copying a divergent API shape.
pub fn json<T: Serialize>(value: T) -> Response {
    Json(value).into_response()
}
pub fn created<T: Serialize>(path: &str, value: T) -> Result<Response, ResponseError> {
    let location = location(path)?;
    let mut response = Json(value).into_response();
    if response.status().is_success() {
        *response.status_mut() = StatusCode::CREATED;
        response.headers_mut().insert(header::LOCATION, location);
    }
    Ok(response)
}
pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}
/// Empty final response; negotiated error defaults are supplied by `finalize`.
pub fn status(code: u16) -> Result<Response, ResponseError> {
    Ok(checked(code)?.into_response())
}
pub fn redirect(path: &str, code: u16) -> Result<Response, ResponseError> {
    if !matches!(code, 301 | 302 | 303 | 307 | 308) {
        return Err(ResponseError::InvalidStatus);
    }
    let mut response = checked(code)?.into_response();
    response
        .headers_mut()
        .insert(header::LOCATION, location(path)?);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
fn quality(accept: &str, kind: &str) -> f32 {
    let mut best = (-1, 0.0);
    for part in accept.split(',').take(64) {
        let mut fields = part.trim().split(';');
        let name = fields.next().unwrap_or("").trim().to_ascii_lowercase();
        let specificity = if name.eq_ignore_ascii_case(kind) {
            2
        } else if name == "*/*" {
            0
        } else if name.ends_with("/*") && kind.starts_with(&name[..name.len() - 1]) {
            1
        } else {
            continue;
        };
        let mut q = 1.0f32;
        for field in fields {
            if let Some((key, value)) = field.trim().split_once('=') {
                if key.eq_ignore_ascii_case("q") {
                    q = value
                        .trim()
                        .parse()
                        .ok()
                        .filter(|v: &f32| v.is_finite() && (0.0..=1.0).contains(v))
                        .unwrap_or(0.0);
                }
            }
        }
        if specificity > best.0 || specificity == best.0 && q > best.1 {
            best = (specificity, q)
        }
    }
    best.1
}
fn html_preferred(path: &str, accept: Option<&str>) -> bool {
    if path == "/api"
        || path.starts_with("/api/")
        || path == "/_brain"
        || path.starts_with("/_brain/")
        || path.starts_with("/_r/")
    {
        return false;
    }
    let accept = accept.filter(|value| value.len() <= 8192).unwrap_or("");
    if accept.is_empty() {
        return true;
    }
    let html = quality(accept, "text/html");
    html > 0.0 && html >= quality(accept, "application/json")
}
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn page(status: StatusCode, rid: &str) -> String {
    let title = status.canonical_reason().unwrap_or("Request failed");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex,nofollow\"><title>{code} — {title}</title></head><body><main><h1>{code} — {title}</h1><p>The request could not be completed. Try again or contact support with this reference.</p><p>Reference: <code>{rid}</code></p><a href=\"/\">Return home</a></main></body></html>",
        code = status.as_u16(),
        rid = escaped(rid)
    )
}
/// Apply after handlers, before response compression where possible. Custom response bodies and semantic headers survive.
pub fn finalize(
    method: &Method,
    path: &str,
    accept: Option<&str>,
    rid: &str,
    mut response: Response,
) -> Response {
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let kind = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let html = html_preferred(path, accept);
        let custom = kind.starts_with("text/html")
            || kind.starts_with("application/json")
            || kind.starts_with("application/problem+json");
        // API JSON stays machine-readable; explicit custom HTML is preserved outside the API namespace.
        if !custom
            || (html && response.extensions().get::<DefaultError>().is_some())
            || (!html
                && kind.starts_with("text/html")
                && (path.starts_with("/api/") || path.starts_with("/_brain/")))
        {
            let body = if html {
                page(status, rid)
            } else {
                serde_json::json!({"schemaVersion":1,"status":status.as_u16(),"rid":rid,"error":{"code":"HTTP_REJECTED","message":status.canonical_reason().unwrap_or("Request failed"),"fix":"Check the documented method, authentication, path and request body"}}).to_string()
            };
            *response.body_mut() = Body::from(body);
            let headers = response.headers_mut();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(if html {
                    "text/html; charset=utf-8"
                } else {
                    "application/json"
                }),
            );
            headers.remove(header::CONTENT_LENGTH);
            headers.remove(header::CONTENT_ENCODING);
            headers.remove(header::ETAG);
        }
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        );
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Accept"));
    }
    if let Ok(value) = HeaderValue::from_str(rid) {
        response.headers_mut().insert("x-request-id", value);
    }
    if method == Method::HEAD
        || status.is_informational()
        || matches!(status.as_u16(), 204 | 205 | 304)
    {
        *response.body_mut() = Body::empty();
        if method != Method::HEAD {
            response.headers_mut().remove(header::CONTENT_LENGTH);
            response.headers_mut().remove(header::CONTENT_TYPE);
            response.headers_mut().remove(header::CONTENT_ENCODING);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    #[tokio::test]
    async fn negotiated_errors_preserve_api_contracts_and_head_has_no_body() {
        let response = finalize(
            &Method::GET,
            "/page",
            Some("text/html"),
            "safe-id",
            StatusCode::FORBIDDEN.into_response(),
        );
        assert_eq!(response.status(), 403);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(
            text.contains("403 — Forbidden")
                && text.contains("viewport")
                && text.contains("safe-id")
        );
        let response = finalize(
            &Method::GET,
            "/api/items",
            Some("*/*"),
            "id",
            (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(serde_json::json!({"error":"custom"})),
            )
                .into_response(),
        );
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "{\"error\":\"custom\"}"
        );
        let response = finalize(
            &Method::HEAD,
            "/page",
            Some("text/html"),
            "id",
            StatusCode::NOT_FOUND.into_response(),
        );
        assert!(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }
    #[test]
    fn helpers_validate_final_codes_and_same_site_locations() {
        for code in [301, 302, 303, 307, 308] {
            assert_eq!(
                redirect("/login?next=%2Fhome", code)
                    .unwrap()
                    .status()
                    .as_u16(),
                code
            )
        }
        for path in [
            "https://evil.test",
            "//evil.test",
            "/\\evil.test",
            "/\r\nLocation: x",
            "/%2fevil.test",
            "/%09/evil",
        ] {
            assert!(redirect(path, 303).is_err(), "{path:?}")
        }
        for code in [99, 100, 199, 600, 999, 299] {
            assert!(status(code).is_err())
        }
        assert_eq!(
            created("/api/items/1", serde_json::json!({"id":1}))
                .unwrap()
                .status(),
            201
        );
        assert_eq!(no_content().status(), 204);
    }
    #[test]
    fn explicit_quality_exclusions_win_over_wildcards() {
        assert!(!html_preferred("/page", Some("text/html;q=0, */*;q=1")));
        assert!(!html_preferred(
            "/page",
            Some("application/json;q=1,text/html;q=0.5")
        ));
        assert!(html_preferred(
            "/page",
            Some("text/html,application/json;q=0.5")
        ));
    }
}
