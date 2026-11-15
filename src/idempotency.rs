//! `Idempotency-Key` support for v1 mutations.
//!
//! The key, the presented credential, the method, and the path identify a
//! cached response. Replaying a key with the same body returns the stored
//! response; reusing it with a different body is rejected with Stripe's
//! `idempotency_error`.

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::time;

const MAX_BODY: usize = 4 * 1024 * 1024;

type CachedResponse = (String, Option<i64>, Option<String>, Option<String>);

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn error_response(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({
            "error": {
                "type": "idempotency_error",
                "message": message,
            }
        })),
    )
        .into_response()
}

fn replay(body: String, status: u16, content_type: Option<String>) -> Response {
    let mut response =
        (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), body).into_response();
    if let Ok(value) = header::HeaderValue::from_str(
        content_type
            .as_deref()
            .unwrap_or("application/json; charset=utf-8"),
    ) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    response.headers_mut().insert(
        "Idempotent-Replay",
        header::HeaderValue::from_static("true"),
    );
    response
}

pub async fn enforce(State(st): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    if !path.starts_with("/v1/") || !matches!(method.as_str(), "POST" | "DELETE" | "PATCH") {
        return next.run(request).await;
    }
    let Some(key) = request
        .headers()
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
    else {
        return next.run(request).await;
    };
    let scope = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| sha256_hex(value.as_bytes()))
        .unwrap_or_else(|| "anonymous".to_string());

    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_response("The request body is too large to buffer".to_string());
        }
    };
    let fingerprint = sha256_hex(&bytes);

    let existing: Option<CachedResponse> = sqlx::query_as(
        "SELECT request_fingerprint, status, body, content_type FROM idempotency_keys
         WHERE idempotency_key = ?1 AND scope = ?2 AND method = ?3 AND path = ?4",
    )
    .bind(&key)
    .bind(&scope)
    .bind(method.as_str())
    .bind(&path)
    .fetch_optional(&st.pool)
    .await
    .ok()
    .flatten();

    if let Some((stored, status, body, content_type)) = existing {
        if stored != fingerprint {
            return error_response(format!(
                "Keys for idempotent requests can only be used with the same parameters they were first used with. Try using a key other than '{key}' if you meant to execute a different request."
            ));
        }
        if let (Some(status), Some(body)) = (status, body) {
            return replay(body, status as u16, content_type);
        }
    }

    let request = Request::from_parts(parts, Body::from(bytes));
    let response = next.run(request).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (parts, body) = response.into_parts();
    let body = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Response::from_parts(parts, Body::empty());
        }
    };
    if status.as_u16() < 500 {
        let _ = sqlx::query(
            "INSERT OR REPLACE INTO idempotency_keys
                (idempotency_key, scope, method, path, request_fingerprint, status, body, content_type, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(&key)
        .bind(&scope)
        .bind(method.as_str())
        .bind(&path)
        .bind(&fingerprint)
        .bind(status.as_u16() as i64)
        .bind(String::from_utf8_lossy(&body).to_string())
        .bind(&content_type)
        .bind(time::now_epoch())
        .execute(&st.pool)
        .await;
    }
    Response::from_parts(parts, Body::from(body))
}
