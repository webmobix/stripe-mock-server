use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

/// Current time as Unix epoch seconds.
pub fn now_epoch() -> i64 {
    Utc::now().timestamp()
}

/// Current time as an RFC 3339 UTC string with millisecond precision.
pub fn now_rfc3339_ms() -> String {
    epoch_to_rfc3339_ms(now_epoch())
}

/// Convert epoch seconds into an RFC 3339 UTC string with millisecond precision.
pub fn epoch_to_rfc3339_ms(epoch: i64) -> String {
    Utc.timestamp_opt(epoch, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Parse an RFC 3339 timestamp into Unix epoch seconds.
pub fn rfc3339_to_epoch(value: &str) -> Result<i64, chrono::ParseError> {
    let dt: DateTime<Utc> = DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc);
    Ok(dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_millisecond_precision() {
        let epoch = 1_760_000_123i64;
        let formatted = epoch_to_rfc3339_ms(epoch);
        assert!(formatted.ends_with("Z"));
        assert!(formatted.contains('.'));
        assert_eq!(formatted, "2025-10-09T08:55:23.000Z");
        assert_eq!(rfc3339_to_epoch(&formatted).unwrap(), epoch);
    }

    #[test]
    fn parses_arbitrary_offsets_to_utc_epoch() {
        let epoch = rfc3339_to_epoch("2025-10-09T08:55:23.500Z").unwrap();
        assert_eq!(epoch, 1_760_000_123);
    }
}
