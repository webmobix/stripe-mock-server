use serde_json::{Value, json};

use crate::time;
use crate::util::{empty_object, list_envelope};

pub fn parse_json(value: &Option<String>) -> Value {
    value
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null)
}

pub fn parse_json_or_empty(value: &Option<String>) -> Value {
    match parse_json(value) {
        Value::Null => json!({}),
        other => other,
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountRow {
    pub id: String,
    pub created: i64,
    pub updated: i64,
    pub livemode: i64,
    pub is_main: i64,
    pub business_type: Option<String>,
    pub email: Option<String>,
    pub country: Option<String>,
    pub default_currency: String,
    pub identity: Option<String>,
    pub configuration: Option<String>,
    pub capabilities: Option<String>,
    pub requirements: Option<String>,
    pub defaults: Option<String>,
    pub details_submitted: i64,
    pub charges_enabled: i64,
    pub payouts_enabled: i64,
    pub metadata: Option<String>,
}

impl AccountRow {
    pub fn applied_configurations(&self) -> Vec<String> {
        match parse_json(&self.configuration) {
            Value::Object(map) => map.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }

    pub fn capabilities(&self) -> Value {
        match parse_json(&self.capabilities) {
            Value::Object(_) => parse_json(&self.capabilities),
            _ => json!({}),
        }
    }

    /// Build the v2 `configuration` object, merging the canonical capability
    /// statuses so activation is visible after onboarding. Capabilities read as
    /// `{"status": "active"|"pending"|"rejected", "status_details": []}`.
    pub fn configuration_v2(&self) -> Value {
        self.configuration_v2_for(&[])
    }

    /// Like `configuration_v2`, limited to the named sections (empty = all).
    pub fn configuration_v2_for(&self, only: &[String]) -> Value {
        let stored = parse_json(&self.configuration);
        let caps = self.capabilities();
        let Value::Object(sections) = stored else {
            return json!({});
        };
        let mut out = serde_json::Map::new();
        for (section, section_value) in sections {
            if !only.is_empty() && !only.contains(&section) {
                continue;
            }
            let mut sec = serde_json::Map::new();
            sec.insert("applied".to_string(), json!(true));
            let mut names: Vec<String> = section_value
                .get("capabilities")
                .and_then(|c| c.as_object())
                .map(|c| c.keys().cloned().collect())
                .unwrap_or_default();
            if let Value::Object(cmap) = &caps {
                for name in cmap.keys() {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
            }
            if !names.is_empty() {
                let mut caps_out = serde_json::Map::new();
                for name in names {
                    let status = caps
                        .get(name.as_str())
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending");
                    caps_out.insert(name, json!({ "status": status, "status_details": [] }));
                }
                sec.insert("capabilities".to_string(), Value::Object(caps_out));
            }
            out.insert(section, Value::Object(sec));
        }
        Value::Object(out)
    }

    /// Serialize as a `v2.core.account`, honoring `include` and default-null semantics.
    pub fn to_v2(&self, include: &[String]) -> Value {
        // `configuration.merchant` includes just that section; `configuration`
        // includes all of them. Other fields match by exact name.
        let has = |name: &str| include.iter().any(|i| i == name);
        let sections: Vec<String> = include
            .iter()
            .filter_map(|i| i.strip_prefix("configuration.").map(str::to_string))
            .collect();
        let with_config = has("configuration") || !sections.is_empty();
        let only: Vec<String> = if has("configuration") {
            Vec::new()
        } else {
            sections
        };
        let mut obj = empty_object();
        obj.insert("id".into(), json!(self.id));
        obj.insert("object".into(), json!("v2.core.account"));
        obj.insert(
            "created".into(),
            json!(time::epoch_to_rfc3339_ms(self.created)),
        );
        obj.insert("livemode".into(), json!(self.livemode != 0));
        obj.insert(
            "applied_configurations".into(),
            json!(self.applied_configurations()),
        );
        obj.insert(
            "identity".into(),
            if has("identity") {
                parse_json(&self.identity)
            } else {
                Value::Null
            },
        );
        obj.insert(
            "requirements".into(),
            if has("requirements") {
                parse_json(&self.requirements)
            } else {
                Value::Null
            },
        );
        obj.insert(
            "configuration".into(),
            if with_config {
                self.configuration_v2_for(&only)
            } else {
                Value::Null
            },
        );
        obj.insert(
            "defaults".into(),
            if has("defaults") {
                parse_json(&self.defaults)
            } else {
                Value::Null
            },
        );
        obj.insert("metadata".into(), parse_json_or_empty(&self.metadata));
        Value::Object(obj)
    }

    /// Serialize as a v1 `account` projection.
    pub fn to_v1(&self) -> Value {
        let requirements = match parse_json(&self.requirements) {
            Value::Object(_) => parse_json(&self.requirements),
            _ => json!({
                "alternatives": [],
                "current_deadline": null,
                "currently_due": [],
                "disabled_reason": null,
                "eventually_due": [],
                "past_due": [],
                "pending_verification": [],
            }),
        };
        let capabilities = self.capabilities();
        json!({
            "id": self.id,
            "object": "account",
            "business_type": self.business_type,
            "charges_enabled": self.charges_enabled != 0,
            "country": self.country,
            "created": self.created,
            "default_currency": self.default_currency,
            "details_submitted": self.details_submitted != 0,
            "email": self.email,
            "livemode": self.livemode != 0,
            "metadata": parse_json_or_empty(&self.metadata),
            "payouts_enabled": self.payouts_enabled != 0,
            "capabilities": capabilities,
            "requirements": requirements,
            "type": "standard",
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProductRow {
    pub id: String,
    pub account_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub active: i64,
    pub created: i64,
    pub default_price: Option<String>,
    pub metadata: Option<String>,
}

impl ProductRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "product",
            "active": self.active != 0,
            "created": self.created,
            "default_price": self.default_price,
            "description": self.description,
            "images": [],
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "name": self.name,
            "type": "service",
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PriceRow {
    pub id: String,
    pub account_id: String,
    pub product_id: String,
    pub currency: String,
    pub unit_amount: Option<i64>,
    pub price_type: String,
    pub recurring_interval: Option<String>,
    pub recurring_interval_count: Option<i64>,
    pub nickname: Option<String>,
    pub active: i64,
    pub created: i64,
    pub metadata: Option<String>,
}

impl PriceRow {
    pub fn to_json(&self) -> Value {
        let recurring = if self.price_type == "recurring" {
            json!({
                "interval": self.recurring_interval,
                "interval_count": self.recurring_interval_count.unwrap_or(1),
                "meter": null,
                "trial_period_days": null,
                "usage_type": "licensed",
            })
        } else {
            Value::Null
        };
        let unit_amount_decimal = self.unit_amount.map(|a| format!("{a}"));
        json!({
            "id": self.id,
            "object": "price",
            "active": self.active != 0,
            "billing_scheme": "per_unit",
            "created": self.created,
            "currency": self.currency,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "nickname": self.nickname,
            "product": self.product_id,
            "recurring": recurring,
            "type": self.price_type,
            "unit_amount": self.unit_amount,
            "unit_amount_decimal": unit_amount_decimal,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CustomerRow {
    pub id: String,
    pub account_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub created: i64,
    pub metadata: Option<String>,
}

impl CustomerRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "customer",
            "created": self.created,
            "email": self.email,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "name": self.name,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SubscriptionRow {
    pub id: String,
    pub account_id: String,
    pub customer: Option<String>,
    pub customer_account: Option<String>,
    pub status: String,
    pub created: i64,
    pub current_period_start: Option<i64>,
    pub current_period_end: Option<i64>,
    pub cancel_at_period_end: i64,
    pub items: String,
    pub metadata: Option<String>,
}

impl SubscriptionRow {
    pub fn to_json(&self) -> Value {
        let items = parse_json(&Some(self.items.clone()));
        json!({
            "id": self.id,
            "object": "subscription",
            "cancel_at_period_end": self.cancel_at_period_end != 0,
            "created": self.created,
            "current_period_start": self.current_period_start,
            "current_period_end": self.current_period_end,
            "customer": self.customer,
            "customer_account": self.customer_account,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "status": self.status,
            "items": {
                "object": "list",
                "data": items,
                "has_more": false,
                "url": format!("/v1/subscription_items?subscription={}", self.id),
            },
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CheckoutSessionRow {
    pub id: String,
    pub account_id: String,
    pub mode: String,
    pub status: String,
    pub created: i64,
    pub expires_at: i64,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub customer: Option<String>,
    pub customer_account: Option<String>,
    pub line_items: String,
    pub payment_intent: Option<String>,
    pub subscription: Option<String>,
    pub amount_total: Option<i64>,
    pub currency: Option<String>,
    pub url: Option<String>,
    pub metadata: Option<String>,
}

impl CheckoutSessionRow {
    pub fn to_json(&self) -> Value {
        let line_items = parse_json(&Some(self.line_items.clone()));
        json!({
            "id": self.id,
            "object": "checkout.session",
            "amount_total": self.amount_total,
            "cancel_url": self.cancel_url,
            "created": self.created,
            "currency": self.currency,
            "customer": self.customer,
            "customer_account": self.customer_account,
            "expires_at": self.expires_at,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "mode": self.mode,
            "payment_intent": self.payment_intent,
            "payment_status": if self.status == "complete" { "paid" } else { "unpaid" },
            "status": self.status,
            "subscription": self.subscription,
            "success_url": self.success_url,
            "url": self.url,
            "line_items": list_envelope(
                &format!("/v1/checkout/sessions/{}/line_items", self.id),
                line_items.as_array().cloned().unwrap_or_default(),
                false,
            ),
        })
    }

    pub fn line_items(&self) -> Vec<Value> {
        parse_json(&Some(self.line_items.clone()))
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PaymentIntentRow {
    pub id: String,
    pub account_id: String,
    pub amount: Option<i64>,
    pub currency: Option<String>,
    pub status: String,
    pub customer: Option<String>,
    pub created: i64,
    pub checkout_session: Option<String>,
    pub metadata: Option<String>,
}

impl PaymentIntentRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "payment_intent",
            "amount": self.amount,
            "created": self.created,
            "currency": self.currency,
            "customer": self.customer,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "status": self.status,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub id: String,
    pub account_id: String,
    pub event_account: Option<String>,
    #[sqlx(rename = "type")]
    pub event_type: String,
    pub created: i64,
    pub api_version: String,
    pub version: String,
    pub data: String,
    pub request: Option<String>,
    pub livemode: i64,
}

impl EventRow {
    /// `pending_webhooks` is the number of deliveries not yet settled.
    pub fn to_json(&self, pending_webhooks: i64) -> Value {
        let mut obj = empty_object();
        obj.insert("id".into(), json!(self.id));
        obj.insert("object".into(), json!("event"));
        obj.insert("api_version".into(), json!(self.api_version));
        obj.insert("created".into(), json!(self.created));
        obj.insert("type".into(), json!(self.event_type));
        obj.insert(
            "data".into(),
            json!({ "object": parse_json(&Some(self.data.clone())) }),
        );
        obj.insert("livemode".into(), json!(self.livemode != 0));
        obj.insert("pending_webhooks".into(), json!(pending_webhooks));
        obj.insert(
            "request".into(),
            json!({ "id": null, "idempotency_key": null }),
        );
        if let Some(account) = &self.event_account {
            obj.insert("account".into(), json!(account));
        }
        Value::Object(obj)
    }

    /// True when this row represents a v2 thin event.
    pub fn is_v2(&self) -> bool {
        self.version == "v2"
    }

    /// The stored v2 envelope (`related_object` plus fetched-only fields).
    fn v2_envelope(&self) -> Value {
        parse_json(&Some(self.data.clone()))
    }

    /// The `related_object.id` for a v2 event, if any.
    pub fn v2_related_id(&self) -> Option<String> {
        self.v2_envelope()
            .get("related_object")
            .and_then(|r| r.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    /// The unversioned thin payload delivered to event destinations.
    pub fn to_v2_thin(&self) -> Value {
        let envelope = self.v2_envelope();
        let mut obj = empty_object();
        obj.insert("id".into(), json!(self.id));
        obj.insert("object".into(), json!("v2.core.event"));
        obj.insert("type".into(), json!(self.event_type));
        obj.insert(
            "created".into(),
            json!(time::epoch_to_rfc3339_ms(self.created)),
        );
        obj.insert("livemode".into(), json!(self.livemode != 0));
        obj.insert(
            "related_object".into(),
            envelope
                .get("related_object")
                .cloned()
                .unwrap_or(Value::Null),
        );
        Value::Object(obj)
    }

    /// The full versioned event returned by `GET /v2/core/events/{id}`.
    pub fn to_v2_full(&self) -> Value {
        let envelope = self.v2_envelope();
        let mut obj = self.to_v2_thin();
        if let Value::Object(map) = &mut obj {
            let field = |name: &str| envelope.get(name).cloned().unwrap_or(Value::Null);
            map.insert("data".into(), field("data"));
            map.insert("reason".into(), field("reason"));
            map.insert("changes".into(), field("changes"));
            map.insert("context".into(), field("context"));
        }
        obj
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeliveryRow {
    pub id: String,
    pub event_id: String,
    pub destination_id: String,
    pub status: String,
    pub attempts: i64,
    pub http_status: Option<i64>,
    pub last_error: Option<String>,
    pub created: i64,
    pub updated: i64,
}

impl DeliveryRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "mock.delivery",
            "event": self.event_id,
            "destination": self.destination_id,
            "status": self.status,
            "attempts": self.attempts,
            "http_status": self.http_status,
            "last_error": self.last_error,
            "created": self.created,
            "updated": self.updated,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventDestinationRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub description: Option<String>,
    pub url: String,
    pub enabled_events: String,
    pub event_payload: String,
    pub events_from: Option<String>,
    pub status: String,
    pub metadata: Option<String>,
    pub snapshot_api_version: Option<String>,
    pub secret: String,
    pub created: i64,
    pub updated: i64,
}

impl EventDestinationRow {
    pub fn enabled_events(&self) -> Vec<String> {
        parse_json(&Some(self.enabled_events.clone()))
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn events_from(&self) -> Vec<String> {
        self.events_from
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| {
                v.as_array().map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_else(|| vec!["@self".to_string()])
    }

    /// Serialize as a `v2.core.event_destination`, honoring include-gated fields.
    pub fn to_json(&self, include_url: bool, include_secret: bool) -> Value {
        let status_details = if self.status == "disabled" {
            json!({ "disabled": { "reason": "user" } })
        } else {
            Value::Null
        };
        let mut obj = empty_object();
        obj.insert("id".into(), json!(self.id));
        obj.insert("object".into(), json!("v2.core.event_destination"));
        obj.insert(
            "created".into(),
            json!(time::epoch_to_rfc3339_ms(self.created)),
        );
        obj.insert(
            "updated".into(),
            json!(time::epoch_to_rfc3339_ms(self.updated)),
        );
        obj.insert("name".into(), json!(self.name));
        obj.insert("description".into(), json!(self.description));
        obj.insert("enabled_events".into(), json!(self.enabled_events()));
        obj.insert("event_payload".into(), json!(self.event_payload));
        obj.insert("events_from".into(), json!(self.events_from()));
        obj.insert("livemode".into(), json!(false));
        obj.insert("metadata".into(), parse_json_or_empty(&self.metadata));
        obj.insert(
            "snapshot_api_version".into(),
            json!(self.snapshot_api_version),
        );
        obj.insert("status".into(), json!(self.status));
        obj.insert("status_details".into(), status_details);
        obj.insert("type".into(), json!("webhook_endpoint"));
        obj.insert(
            "webhook_endpoint".into(),
            json!({
                "url": if include_url { json!(self.url) } else { Value::Null },
                "signing_secret": if include_secret { json!(self.secret) } else { Value::Null },
            }),
        );
        Value::Object(obj)
    }

    /// Internal representation including the URL and secret unconditionally.
    pub fn to_internal_json(&self) -> Value {
        self.to_json(true, true)
    }
}
