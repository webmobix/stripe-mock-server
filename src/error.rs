use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiVersion {
    V1,
    V2,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub version: ApiVersion,
    pub status: StatusCode,
    pub error_type: String,
    pub code: Option<String>,
    pub message: String,
    pub param: Option<String>,
}

impl ApiError {
    pub fn new(
        version: ApiVersion,
        status: StatusCode,
        error_type: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            version,
            status,
            error_type: error_type.to_string(),
            code: None,
            message: message.into(),
            param: None,
        }
    }

    pub fn with_code(mut self, code: &str) -> Self {
        self.code = Some(code.to_string());
        self
    }

    pub fn with_param(mut self, param: &str) -> Self {
        self.param = Some(param.to_string());
        self
    }

    pub fn invalid_request(version: ApiVersion, message: impl Into<String>) -> Self {
        Self::new(
            version,
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            message,
        )
        .with_code("parameter_invalid")
    }

    pub fn invalid_param(version: ApiVersion, param: &str, message: impl Into<String>) -> Self {
        Self::invalid_request(version, message).with_param(param)
    }

    pub fn missing_param(version: ApiVersion, param: &str) -> Self {
        Self::invalid_request(version, format!("Missing required param: {param}"))
            .with_code("parameter_missing")
            .with_param(param)
    }

    pub fn auth(version: ApiVersion, message: impl Into<String>) -> Self {
        Self::auth_with_detail(version, message, None)
    }

    /// Like [`ApiError::auth`], but `detail` is extra context (for example a
    /// redacted key) that is written to the log only, never to the client. Every
    /// authentication failure is logged as a warning here, in one place.
    pub fn auth_with_detail(
        version: ApiVersion,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        let message = message.into();
        match detail {
            Some(detail) => tracing::warn!("authentication failed: {message} ({detail})"),
            None => tracing::warn!("authentication failed: {message}"),
        }
        Self::new(
            version,
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            message,
        )
        .with_code("api_key_invalid")
    }

    pub fn not_found(version: ApiVersion, message: impl Into<String>) -> Self {
        Self::new(
            version,
            StatusCode::NOT_FOUND,
            "invalid_request_error",
            message,
        )
        .with_code("resource_missing")
    }

    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::internal_for(ApiVersion::V1, message)
    }

    pub fn internal_v2(message: impl std::fmt::Display) -> Self {
        Self::internal_for(ApiVersion::V2, message)
    }

    /// The real cause is logged; the client only sees a generic message.
    pub fn internal_for(version: ApiVersion, message: impl std::fmt::Display) -> Self {
        tracing::error!("internal API error: {message}");
        Self::new(
            version,
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_error",
            "An unexpected error occurred.",
        )
        .with_code("api_error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match self.version {
            ApiVersion::V1 => {
                let mut error = serde_json::Map::new();
                error.insert("type".to_string(), json!(self.error_type));
                error.insert("message".to_string(), json!(self.message));
                if let Some(code) = self.code {
                    error.insert("code".to_string(), json!(code));
                }
                if let Some(param) = self.param {
                    error.insert("param".to_string(), json!(param));
                }
                json!({ "error": Value::Object(error) })
            }
            ApiVersion::V2 => {
                let mut error = super::util::empty_object();
                error.insert("type".to_string(), json!(self.error_type));
                error.insert(
                    "code".to_string(),
                    json!(self.code.unwrap_or_else(|| "invalid_request".to_string())),
                );
                error.insert("message".to_string(), json!(self.message));
                json!({ "error": Value::Object(error) })
            }
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_envelope_includes_param() {
        let err = ApiError::missing_param(ApiVersion::V1, "name");
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn v2_envelope_has_code() {
        let err = ApiError::invalid_request(ApiVersion::V2, "bad value");
        assert_eq!(err.version, ApiVersion::V2);
        assert_eq!(err.code.as_deref(), Some("parameter_invalid"));
    }
}
