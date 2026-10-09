pub mod v1;
pub mod v2;

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

/// Wrap a JSON value into an axum response.
pub fn ok(value: Value) -> Response {
    Json(value).into_response()
}

/// Read a nested string field from a form- or JSON-parsed value.
pub fn get_str(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// Read a nested integer field, accepting numeric JSON or numeric strings.
pub fn get_i64(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

/// Read a nested boolean field, accepting booleans and `"true"`/`"false"`.
pub fn get_bool(value: &Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Read a nested array field.
pub fn get_array<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Vec<Value>> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_array()
}

/// Parse a query string into decoded key/value pairs, preserving duplicates.
pub fn parse_query_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |s: &str| crate::util::form_decode(s);
            (decode(k), decode(v))
        })
        .collect()
}

/// Collect repeated `include`/`include[]` query values.
/// `include`, `include[]`, or the indexed form `include[0]` that stripe-node emits.
fn is_include_key(key: &str) -> bool {
    match key.strip_prefix("include") {
        Some("") | Some("[]") => true,
        Some(rest) => rest
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .is_some_and(|idx| !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit())),
        None => false,
    }
}

pub fn collect_includes(query: &str) -> Vec<String> {
    parse_query_pairs(query)
        .into_iter()
        .filter(|(k, _)| is_include_key(k))
        .flat_map(|(_, v)| v.split(',').map(str::to_string).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Apply `limit`/`starting_after`/`ending_before` pagination to a newest-first list.
pub fn paginate<T: Clone>(
    mut items: Vec<T>,
    limit: Option<i64>,
    starting_after: Option<String>,
    ending_before: Option<String>,
    id_of: impl Fn(&T) -> String,
) -> (Vec<T>, bool) {
    if let Some(after) = starting_after {
        match items.iter().position(|i| id_of(i) == after) {
            Some(pos) => items = items.into_iter().skip(pos + 1).collect(),
            None => items.clear(),
        }
    }
    let limit = limit.unwrap_or(10).clamp(1, 100) as usize;
    if let Some(before) = ending_before {
        // Items are newest-first: the page is the `limit` items right before the cursor.
        match items.iter().position(|i| id_of(i) == before) {
            Some(pos) => {
                items.truncate(pos);
                let has_more = items.len() > limit;
                let start = items.len().saturating_sub(limit);
                return (items.split_off(start), has_more);
            }
            None => return (Vec::new(), false),
        }
    }
    let has_more = items.len() > limit;
    items.truncate(limit);
    (items, has_more)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_flat_and_nested_fields() {
        let value = json!({"a": "x", "b": {"c": 3}, "d": "true", "e": [1,2]});
        assert_eq!(get_str(&value, &["a"]).as_deref(), Some("x"));
        assert_eq!(get_i64(&value, &["b", "c"]), Some(3));
        assert_eq!(get_bool(&value, &["d"]), Some(true));
        assert_eq!(get_array(&value, &["e"]).unwrap().len(), 2);
    }

    #[test]
    fn collects_repeated_includes() {
        let includes = collect_includes("include[]=identity&include[]=requirements&limit=5");
        assert_eq!(
            includes,
            vec!["identity".to_string(), "requirements".to_string()]
        );
    }

    #[test]
    fn paginates_newest_first() {
        let items: Vec<String> = (0..5).map(|i| format!("id{i}")).collect();
        let (page, has_more) = paginate(items.clone(), Some(2), None, None, |s| s.clone());
        assert_eq!(page, vec!["id0", "id1"]);
        assert!(has_more);
        let (page, has_more) = paginate(items, Some(2), Some("id1".into()), None, |s| s.clone());
        assert_eq!(page, vec!["id2", "id3"]);
        assert!(has_more);
    }

    #[test]
    fn ending_before_returns_page_just_before_cursor() {
        let items: Vec<String> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (page, has_more) = paginate(items.clone(), Some(2), None, Some("e".into()), |s| {
            s.clone()
        });
        assert_eq!(page, vec!["c", "d"]);
        assert!(has_more);
        let (page, has_more) = paginate(items, Some(2), None, Some("c".into()), |s| s.clone());
        assert_eq!(page, vec!["a", "b"]);
        assert!(!has_more);
    }
}

/// JSON body extractor for v2 routes: an empty body is `{}` and a malformed
/// one produces a v2-shaped error instead of axum's plain-text rejection.
pub struct V2Json(pub Value);

impl<S: Send + Sync> axum::extract::FromRequest<S> for V2Json {
    type Rejection = crate::error::ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        use crate::error::{ApiError, ApiVersion};
        let bytes = axum::body::Bytes::from_request(req, state)
            .await
            .map_err(|e| {
                ApiError::invalid_request(
                    ApiVersion::V2,
                    format!("Could not read request body: {e}"),
                )
            })?;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(V2Json(serde_json::json!({})));
        }
        serde_json::from_slice(&bytes).map(V2Json).map_err(|e| {
            ApiError::invalid_request(ApiVersion::V2, format!("Invalid JSON body: {e}"))
        })
    }
}
