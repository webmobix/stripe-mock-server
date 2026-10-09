use serde_json::{Map, Value, json};

/// An empty `serde_json::Map`, convenient for building response objects.
pub fn empty_object() -> Map<String, Value> {
    Map::new()
}

/// Build a Stripe list envelope.
pub fn list_envelope(url: &str, data: Vec<Value>, has_more: bool) -> Value {
    json!({
        "object": "list",
        "data": data,
        "has_more": has_more,
        "url": url,
    })
}

/// Build a v2 list envelope, which uses `next_page_url` instead of `has_more`.
pub fn list_envelope_v2(
    url: &str,
    data: Vec<Value>,
    has_more: bool,
    next_page_url: Option<String>,
) -> Value {
    let mut obj = empty_object();
    obj.insert("object".to_string(), json!("list"));
    obj.insert("data".to_string(), json!(data));
    obj.insert("has_more".to_string(), json!(has_more));
    obj.insert("next_page_url".to_string(), json!(next_page_url));
    obj.insert("url".to_string(), json!(url));
    Value::Object(obj)
}

/// Decode an `application/x-www-form-urlencoded` component (`+` is a space).
pub fn form_decode(raw: &str) -> String {
    let spaced = raw.replace('+', " ");
    urlencoding::decode(&spaced)
        .map(|c| c.into_owned())
        .unwrap_or(spaced)
}

/// Escape text for safe interpolation into HTML content or attribute values.
pub fn html_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_decode_turns_plus_into_space() {
        assert_eq!(form_decode("Acme+Corp%2B1"), "Acme Corp+1");
    }

    #[test]
    fn html_escape_neutralizes_markup() {
        assert_eq!(html_escape(r#""><script>"#), "&quot;&gt;&lt;script&gt;");
    }
}
