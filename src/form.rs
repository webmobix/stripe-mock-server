use serde_json::{Map, Value, json};

/// Parse an `application/x-www-form-urlencoded` body, expanding Stripe's
/// bracketed notation into nested JSON.
///
/// Examples:
/// - `amount=1000&currency=usd` -> flat object
/// - `line_items[0][price]=price_123` -> `{ "line_items": [ { "price": "price_123" } ] }`
/// - `items[][price]=a&items[][price]=b` -> `{ "items": [ { "price": "a" }, { "price": "b" } ] }`
/// - a repeated flat key keeps the last value
pub fn parse_form(body: &str) -> Value {
    let mut root = Value::Object(Map::new());
    for pair in body.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        let key = crate::util::form_decode(raw_key);
        let value = crate::util::form_decode(raw_value);
        assign(&mut root, &tokenize(&key), Value::String(value));
    }
    root
}

/// Largest array index accepted; guards against huge sparse-array allocations.
const MAX_INDEX: usize = 1000;

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Key(String),
    Index(usize),
    Push,
}

fn tokenize(key: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let bytes = key.as_bytes();
    let mut i = 0;
    let mut first = true;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b']' {
                j += 1;
            }
            let inner = &key[start..j.min(bytes.len())];
            if inner.is_empty() {
                segments.push(Segment::Push);
            } else if let Ok(index) = inner.parse::<usize>() {
                segments.push(Segment::Index(index));
            } else {
                segments.push(Segment::Key(inner.to_string()));
            }
            i = if j < bytes.len() { j + 1 } else { j };
        } else if first {
            let start = i;
            let mut j = i;
            while j < bytes.len() && bytes[j] != b'[' {
                j += 1;
            }
            segments.push(Segment::Key(key[start..j].to_string()));
            first = false;
            i = j;
        } else {
            i += 1;
        }
    }
    if segments.is_empty() {
        segments.push(Segment::Key(key.to_string()));
    }
    segments
}

fn assign(slot: &mut Value, segments: &[Segment], value: Value) {
    match segments.split_first() {
        None => {
            *slot = value;
        }
        Some((Segment::Key(key), rest)) => {
            if !slot.is_object() {
                *slot = json!({});
            }
            let object = slot.as_object_mut().expect("object");
            let entry = object.entry(key.clone()).or_insert(Value::Null);
            assign(entry, rest, value);
        }
        Some((Segment::Index(index), rest)) => {
            if !slot.is_array() {
                *slot = json!([]);
            }
            if *index > MAX_INDEX {
                return;
            }
            let array = slot.as_array_mut().expect("array");
            while array.len() <= *index {
                array.push(Value::Null);
            }
            assign(&mut array[*index], rest, value);
        }
        Some((Segment::Push, rest)) => {
            if !slot.is_array() {
                *slot = json!([]);
            }
            let array = slot.as_array_mut().expect("array");
            // Fields of one `[]` element are grouped: reuse the last element
            // unless it already holds the key being assigned.
            let reuse = match (rest.first(), array.last()) {
                (Some(Segment::Key(key)), Some(Value::Object(last))) => !last.contains_key(key),
                _ => false,
            };
            if !reuse {
                array.push(Value::Null);
            }
            let last = array.len() - 1;
            assign(&mut array[last], rest, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_pairs() {
        let value = parse_form("amount=1000&currency=usd");
        assert_eq!(value["amount"], "1000");
        assert_eq!(value["currency"], "usd");
    }

    #[test]
    fn parses_nested_index_notation() {
        let value =
            parse_form("line_items%5B0%5D%5Bprice%5D=price_1&line_items%5B0%5D%5Bquantity%5D=2");
        assert_eq!(value["line_items"][0]["price"], "price_1");
        assert_eq!(value["line_items"][0]["quantity"], "2");
    }

    #[test]
    fn parses_bracketed_without_encoding() {
        let value = parse_form("line_items[0][price]=price_1&line_items[1][price]=price_2");
        assert_eq!(value["line_items"][0]["price"], "price_1");
        assert_eq!(value["line_items"][1]["price"], "price_2");
    }

    #[test]
    fn parses_repeated_push_keys() {
        let value = parse_form("items[][price]=a&items[][price]=b");
        assert_eq!(value["items"][0]["price"], "a");
        assert_eq!(value["items"][1]["price"], "b");
    }

    #[test]
    fn groups_push_fields_into_one_element() {
        let value = parse_form(
            "items[][price]=p1&items[][quantity]=2&items[][price]=p2&items[][quantity]=3",
        );
        assert_eq!(value["items"].as_array().unwrap().len(), 2);
        assert_eq!(value["items"][0]["quantity"], "2");
        assert_eq!(value["items"][1]["price"], "p2");
    }

    #[test]
    fn huge_index_is_ignored() {
        let value = parse_form("a[4000000000][b]=x&ok=1");
        assert_eq!(value["ok"], "1");
    }

    #[test]
    fn parses_deeply_nested_arrays() {
        let value = parse_form("line_items[data][0][price]=price_1");
        assert_eq!(value["line_items"]["data"][0]["price"], "price_1");
    }

    #[test]
    fn repeated_flat_keys_keep_last() {
        let value = parse_form("a=1&a=2");
        assert_eq!(value["a"], "2");
    }
}
