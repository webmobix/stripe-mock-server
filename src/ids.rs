use rand::Rng;
use sha2::{Digest, Sha256};

const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Generate a random alphanumeric string of length `n`.
pub fn random_string(n: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| ALPHANUMERIC[rng.gen_range(0..ALPHANUMERIC.len())] as char)
        .collect()
}

/// Generate an object identifier with the supplied Stripe prefix.
pub fn id(prefix: &str) -> String {
    format!("{prefix}{}", random_string(24))
}

pub fn account_id() -> String {
    id("acct_")
}

pub fn product_id() -> String {
    id("prod_")
}

pub fn price_id() -> String {
    id("price_")
}

pub fn customer_id() -> String {
    id("cus_")
}

pub fn subscription_id() -> String {
    id("sub_")
}

pub fn checkout_session_id() -> String {
    id("cs_test_")
}

pub fn payment_intent_id() -> String {
    id("pi_")
}

pub fn event_id() -> String {
    id("evt_")
}

/// v2 thin event identifier.
pub fn thin_event_id() -> String {
    id("evt_test_")
}

/// v2 event destination identifier.
pub fn event_destination_id() -> String {
    id("ed_test_")
}

pub fn item_id() -> String {
    id("ii_")
}

pub fn secret_key() -> String {
    format!("sk_test_{}", random_string(24))
}

pub fn publishable_key() -> String {
    format!("pk_test_{}", random_string(24))
}

/// Signing secret for a webhook endpoint URL.
///
/// The mock issues **one secret per endpoint URL**, not one per destination: any
/// destinations that share a `webhook_endpoint.url` (for example a snapshot and a
/// thin destination pointed at the same route) report the same `whsec_` secret.
/// Deriving it deterministically from the URL keeps that guarantee stable across
/// restarts, re-creates, and config re-seeding.
pub fn webhook_secret_for_url(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    format!("whsec_{}", &hex::encode(digest)[..32])
}

pub fn token() -> String {
    random_string(32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn prefixes_match() {
        assert!(account_id().starts_with("acct_"));
        assert!(product_id().starts_with("prod_"));
        assert!(price_id().starts_with("price_"));
        assert!(customer_id().starts_with("cus_"));
        assert!(subscription_id().starts_with("sub_"));
        assert!(checkout_session_id().starts_with("cs_test_"));
        assert!(payment_intent_id().starts_with("pi_"));
        assert!(event_id().starts_with("evt_"));
        assert!(thin_event_id().starts_with("evt_test_"));
        assert!(event_destination_id().starts_with("ed_test_"));
        assert!(secret_key().starts_with("sk_test_"));
        assert!(publishable_key().starts_with("pk_test_"));
        assert!(webhook_secret_for_url("http://localhost:4000/hook").starts_with("whsec_"));
        assert_eq!(
            webhook_secret_for_url("http://localhost:4000/hook"),
            webhook_secret_for_url("http://localhost:4000/hook")
        );
        assert_ne!(
            webhook_secret_for_url("http://localhost:4000/a"),
            webhook_secret_for_url("http://localhost:4000/b")
        );
    }

    #[test]
    fn generated_ids_are_unique() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(account_id()));
        }
    }
}
