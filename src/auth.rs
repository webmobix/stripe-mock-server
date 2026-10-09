use axum::http::HeaderMap;
use base64::Engine;
use sqlx::SqlitePool;

use crate::error::{ApiError, ApiVersion};

#[derive(Debug, Clone)]
pub struct AuthContext {
    /// The account that owns the presented API key.
    pub key_account_id: String,
    /// The account the request acts on (may differ via `Stripe-Account`).
    pub account_id: String,
    /// True when the key belongs to the platform (main) account.
    pub key_is_main: bool,
    /// True when the request acts on a connected account rather than the platform.
    pub is_connected: bool,
    pub version: ApiVersion,
}

impl AuthContext {
    /// A platform key may act on any account; any other key only on its own.
    pub fn can_access_account(&self, account_id: &str) -> bool {
        self.key_is_main || self.key_account_id == account_id
    }
}

/// Extract the API key from an `Authorization` header.
pub fn extract_key(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    if let Some(rest) = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
    {
        return Some(rest.trim().to_string());
    }
    if let Some(rest) = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))
    {
        let rest = rest.trim();
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(rest) {
            if let Ok(text) = String::from_utf8(decoded) {
                let key = text.split(':').next().unwrap_or("").trim().to_string();
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }
        return Some(rest.to_string());
    }
    Some(value.trim().to_string())
}

/// Show only a short prefix of a presented key so failed auth attempts are
/// identifiable in logs without dumping the whole secret.
fn redact_key(key: &str) -> String {
    let prefix: String = key.chars().take(8).collect();
    if key.chars().count() > 8 {
        format!("{prefix}…")
    } else {
        prefix
    }
}

/// Resolve the authenticated account and (optionally) the `Stripe-Account` target.
pub async fn authenticate(
    pool: &SqlitePool,
    headers: &HeaderMap,
    version: ApiVersion,
) -> Result<AuthContext, ApiError> {
    let key = extract_key(headers)
        .ok_or_else(|| ApiError::auth(version, "You did not provide an API key."))?;
    let key_account_id: Option<String> =
        sqlx::query_scalar("SELECT account_id FROM api_keys WHERE key = ?1 AND kind = 'secret'")
            .bind(&key)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::internal_for(version, e))?;
    let key_account_id = key_account_id.ok_or_else(|| {
        ApiError::auth_with_detail(
            version,
            "Invalid API Key provided.",
            Some(format!("presented key {}", redact_key(&key))),
        )
    })?;

    let key_is_main: i64 = sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
        .bind(&key_account_id)
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::internal_for(version, e))?;

    let stripe_account = headers
        .get("stripe-account")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let account_id = match stripe_account {
        None => key_account_id.clone(),
        Some(target) if target == key_account_id => key_account_id.clone(),
        Some(target) => {
            let exists: Option<i64> =
                sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
                    .bind(&target)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| ApiError::internal_for(version, e))?;
            match exists {
                None => {
                    return Err(ApiError::auth(
                        version,
                        format!(
                            "The account {target} does not exist or is not connected to your platform."
                        ),
                    ));
                }
                Some(_) if key_is_main != 0 => target,
                Some(_) => {
                    return Err(ApiError::auth(
                        version,
                        format!("The account {target} is not connected to your account."),
                    ));
                }
            }
        }
    };

    let account_is_main: i64 = sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
        .bind(&account_id)
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::internal_for(version, e))?;

    Ok(AuthContext {
        key_account_id,
        account_id,
        key_is_main: key_is_main != 0,
        is_connected: account_is_main == 0,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn extracts_bearer_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer sk_test_abc"),
        );
        assert_eq!(extract_key(&headers).as_deref(), Some("sk_test_abc"));
    }

    #[test]
    fn extracts_basic_base64_key() {
        let encoded = base64::engine::general_purpose::STANDARD.encode("sk_test_abc:");
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Basic {encoded}")).unwrap(),
        );
        assert_eq!(extract_key(&headers).as_deref(), Some("sk_test_abc"));
    }

    #[test]
    fn extracts_raw_basic_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Basic sk_test_abc"),
        );
        assert_eq!(extract_key(&headers).as_deref(), Some("sk_test_abc"));
    }

    #[test]
    fn redacts_long_keys() {
        assert_eq!(redact_key("sk_test_abcdefghij"), "sk_test_…");
        assert_eq!(redact_key("short"), "short");
    }
}
