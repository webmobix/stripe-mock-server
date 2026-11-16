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
    pub lookup_key: Option<String>,
    pub billing_scheme: String,
    pub tiers: Option<String>,
    pub tiers_mode: Option<String>,
    pub usage_type: Option<String>,
    pub meter_id: Option<String>,
}

impl PriceRow {
    /// `licensed` unless the price explicitly tracks usage.
    pub fn usage_type(&self) -> &str {
        self.usage_type.as_deref().unwrap_or("licensed")
    }

    pub fn is_metered(&self) -> bool {
        self.usage_type() == "metered"
    }

    /// The stored pricing tiers, if this is a tiered price.
    pub fn tier_list(&self) -> Vec<Value> {
        parse_json(&self.tiers)
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> Value {
        let recurring = if self.price_type == "recurring" {
            json!({
                "interval": self.recurring_interval,
                "interval_count": self.recurring_interval_count.unwrap_or(1),
                "meter": self.meter_id,
                "trial_period_days": null,
                "usage_type": self.usage_type(),
            })
        } else {
            Value::Null
        };
        let unit_amount_decimal = self.unit_amount.map(|a| format!("{a}"));
        let tiers = if self.billing_scheme == "tiered" {
            Value::Array(self.tier_list())
        } else {
            Value::Null
        };
        json!({
            "id": self.id,
            "object": "price",
            "active": self.active != 0,
            "billing_scheme": self.billing_scheme,
            "created": self.created,
            "currency": self.currency,
            "livemode": false,
            "lookup_key": self.lookup_key,
            "metadata": parse_json_or_empty(&self.metadata),
            "nickname": self.nickname,
            "product": self.product_id,
            "recurring": recurring,
            "tiers": tiers,
            "tiers_mode": self.tiers_mode,
            "type": self.price_type,
            "unit_amount": self.unit_amount,
            "unit_amount_decimal": unit_amount_decimal,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TestClockRow {
    pub id: String,
    pub account_id: String,
    pub name: Option<String>,
    pub frozen_time: i64,
    pub status: String,
    pub created: i64,
    pub deletes_after: i64,
}

impl TestClockRow {
    pub fn to_json(&self) -> Value {
        self.render(&self.status, Value::Null)
    }

    /// Render the clock with an explicit status and `status_details.advancing`.
    ///
    /// The advance endpoint answers `advancing` before its committed `ready`
    /// state, so the same row renders both.
    pub fn render(&self, status: &str, advancing: Value) -> Value {
        json!({
            "id": self.id,
            "object": "test_helpers.test_clock",
            "created": self.created,
            "deletes_after": self.deletes_after,
            "frozen_time": self.frozen_time,
            "livemode": false,
            "name": self.name,
            "status": status,
            "status_details": {
                "advancing": advancing,
                "internal_failure": Value::Null,
            },
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CustomerRow {
    pub id: String,
    pub account_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub currency: Option<String>,
    pub created: i64,
    pub tax_ids: Option<String>,
    pub metadata: Option<String>,
    pub invoice_settings: Option<String>,
    pub test_clock: Option<String>,
}

impl CustomerRow {
    /// The customer's default payment method id, when one is attached.
    pub fn default_payment_method(&self) -> Option<String> {
        parse_json(&self.invoice_settings)
            .get("default_payment_method")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn to_json(&self) -> Value {
        let invoice_settings = parse_json(&self.invoice_settings);
        json!({
            "id": self.id,
            "object": "customer",
            "created": self.created,
            "currency": self.currency,
            "email": self.email,
            "invoice_settings": {
                "custom_fields": invoice_settings.get("custom_fields").cloned().unwrap_or(Value::Null),
                "default_payment_method": self.default_payment_method(),
                "footer": invoice_settings.get("footer").cloned().unwrap_or(Value::Null),
                "rendering_options": invoice_settings.get("rendering_options").cloned().unwrap_or(Value::Null),
            },
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "name": self.name,
            "tax_ids": parse_json_or_empty(&self.tax_ids),
            "test_clock": self.test_clock,
        })
    }

    /// The stored tax ids, always as a list.
    pub fn tax_id_list(&self) -> Vec<Value> {
        match parse_json(&self.tax_ids) {
            Value::Array(items) => items,
            _ => Vec::new(),
        }
    }
}

/// Build a `tax_id` entry for a customer.
pub fn tax_id_entry(id: &str, tax_type: &str, value: &str, created: i64) -> Value {
    json!({
        "id": id,
        "object": "tax_id",
        "created": created,
        "livemode": false,
        "type": tax_type,
        "value": value,
    })
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PaymentMethodRow {
    pub id: String,
    pub account_id: String,
    pub customer: Option<String>,
    pub pm_type: String,
    pub card: Option<String>,
    pub billing_details: Option<String>,
    pub metadata: Option<String>,
    pub created: i64,
}

impl PaymentMethodRow {
    pub fn to_json(&self) -> Value {
        let mut obj = empty_object();
        obj.insert("id".into(), json!(self.id));
        obj.insert("object".into(), json!("payment_method"));
        obj.insert("type".into(), json!(self.pm_type));
        obj.insert("created".into(), json!(self.created));
        obj.insert("livemode".into(), json!(false));
        obj.insert("customer".into(), json!(self.customer));
        obj.insert("customer_account".into(), Value::Null);
        obj.insert("allow_redisplay".into(), json!("unspecified"));
        obj.insert(
            "billing_details".into(),
            match parse_json(&self.billing_details) {
                Value::Object(map) => Value::Object(map),
                _ => json!({
                    "address": null,
                    "email": null,
                    "name": null,
                    "phone": null,
                }),
            },
        );
        obj.insert("metadata".into(), parse_json_or_empty(&self.metadata));
        if self.pm_type == "card" {
            obj.insert("card".into(), parse_json(&self.card));
        }
        Value::Object(obj)
    }
}

/// Build a card payment method payload for the mock's checkout card.
pub fn card_payment_method_card(brand: &str, last4: &str, exp_month: i64, exp_year: i64) -> Value {
    json!({
        "brand": brand,
        "checks": null,
        "country": "US",
        "display_brand": brand,
        "exp_month": exp_month,
        "exp_year": exp_year,
        "fingerprint": null,
        "funding": "credit",
        "generated_from": null,
        "last4": last4,
        "networks": null,
        "regulated_status": null,
        "three_d_secure_usage": null,
        "wallet": null,
    })
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CouponRow {
    pub id: String,
    pub account_id: String,
    pub percent_off: Option<f64>,
    pub amount_off: Option<i64>,
    pub currency: Option<String>,
    pub duration: String,
    pub duration_in_months: Option<i64>,
    pub max_redemptions: Option<i64>,
    pub times_redeemed: i64,
    pub redeem_by: Option<i64>,
    pub name: Option<String>,
    pub applies_to: Option<String>,
    pub metadata: Option<String>,
    pub created: i64,
}

impl CouponRow {
    /// A coupon is valid while its `redeem_by` has not passed and it has not
    /// reached `max_redemptions` when one is set.
    pub fn is_valid(&self, now: i64) -> bool {
        let not_expired = self.redeem_by.is_none_or(|deadline| deadline >= now);
        let has_redemptions_left = self
            .max_redemptions
            .is_none_or(|max| self.times_redeemed < max);
        not_expired && has_redemptions_left
    }

    pub fn applies_to_products(&self) -> Vec<String> {
        match parse_json(&self.applies_to) {
            Value::Object(mut map) => match map.remove("products") {
                Some(Value::Array(items)) => items
                    .into_iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "coupon",
            "amount_off": self.amount_off,
            "applies_to": { "products": self.applies_to_products() },
            "created": self.created,
            "currency": self.currency,
            "duration": self.duration,
            "duration_in_months": self.duration_in_months,
            "livemode": false,
            "max_redemptions": self.max_redemptions,
            "metadata": parse_json_or_empty(&self.metadata),
            "name": self.name,
            "percent_off": self.percent_off.map(canonical_number),
            "redeem_by": self.redeem_by,
            "times_redeemed": self.times_redeemed,
            "valid": self.is_valid(time::now_epoch()),
        })
    }
}

/// Render a percentage-like number the way Stripe does: whole numbers stay
/// integers and fractional values keep their decimals.
fn canonical_number(value: f64) -> Value {
    if value.fract() == 0.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaxRateRow {
    pub id: String,
    pub account_id: String,
    pub active: i64,
    pub display_name: String,
    pub percentage: f64,
    pub inclusive: i64,
    pub country: Option<String>,
    pub state: Option<String>,
    pub jurisdiction: Option<String>,
    pub description: Option<String>,
    pub tax_type: Option<String>,
    pub metadata: Option<String>,
    pub created: i64,
}

impl TaxRateRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "tax_rate",
            "active": self.active != 0,
            "country": self.country,
            "created": self.created,
            "description": self.description,
            "display_name": self.display_name,
            "inclusive": self.inclusive != 0,
            "jurisdiction": self.jurisdiction,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "percentage": canonical_number(self.percentage),
            "state": self.state,
            "tax_type": self.tax_type,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BillingPortalSessionRow {
    pub id: String,
    pub account_id: String,
    pub customer: String,
    pub return_url: String,
    pub url: String,
    pub created: i64,
}

impl BillingPortalSessionRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "billing_portal.session",
            "created": self.created,
            "customer": self.customer,
            "livemode": false,
            "return_url": self.return_url,
            "url": self.url,
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
    pub test_clock: Option<String>,
    pub latest_invoice: Option<String>,
    pub default_payment_method: Option<String>,
    pub cancel_at: Option<i64>,
    pub canceled_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub billing_cycle_anchor_config: Option<String>,
}

impl SubscriptionRow {
    pub fn to_json(&self) -> Value {
        let items = parse_json(&Some(self.items.clone()));
        // Items stored by an older build lack their own period fields; fill
        // them from the parent row so no JSON migration is needed.
        let items = match items {
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|mut item| {
                        if let Value::Object(map) = &mut item {
                            map.entry("subscription")
                                .or_insert_with(|| json!(self.id.clone()));
                            map.entry("current_period_start")
                                .or_insert(json!(self.current_period_start));
                            map.entry("current_period_end")
                                .or_insert(json!(self.current_period_end));
                        }
                        item
                    })
                    .collect(),
            ),
            other => other,
        };
        json!({
            "id": self.id,
            "object": "subscription",
            "cancel_at_period_end": self.cancel_at_period_end != 0,
            "cancel_at": self.cancel_at,
            "canceled_at": self.canceled_at,
            "created": self.created,
            "current_period_start": self.current_period_start,
            "current_period_end": self.current_period_end,
            "customer": self.customer,
            "customer_account": self.customer_account,
            "default_payment_method": self.default_payment_method,
            "ended_at": self.ended_at,
            "latest_invoice": self.latest_invoice,
            "billing_cycle_anchor_config": parse_json(&self.billing_cycle_anchor_config),
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "status": self.status,
            "test_clock": self.test_clock,
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
    pub customer_email: Option<String>,
    pub line_items: String,
    pub payment_intent: Option<String>,
    pub subscription: Option<String>,
    pub amount_subtotal: Option<i64>,
    pub amount_total: Option<i64>,
    pub amount_discount: Option<i64>,
    pub amount_tax: Option<i64>,
    pub discounts: Option<String>,
    pub currency: Option<String>,
    pub url: Option<String>,
    pub metadata: Option<String>,
    pub ui_mode: String,
    pub return_url: Option<String>,
    pub client_secret: Option<String>,
    pub subscription_metadata: Option<String>,
    pub payment_method_collection: String,
    pub automatic_tax: Option<String>,
}

impl CheckoutSessionRow {
    pub fn is_embedded(&self) -> bool {
        self.ui_mode == "embedded_page" || self.ui_mode == "elements"
    }

    /// Metadata applied to the subscription created on completion.
    pub fn subscription_metadata(&self) -> Value {
        match parse_json(&self.subscription_metadata) {
            Value::Object(map) => Value::Object(map),
            _ => json!({}),
        }
    }

    /// The applied coupon snapshots stored on the session.
    pub fn discounts(&self) -> Vec<Value> {
        parse_json(&self.discounts)
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// The echoed `automatic_tax` object: the stored enabled flag plus the
    /// currently unsupported automatic-tax `status`.
    fn automatic_tax(&self) -> Value {
        let enabled = parse_json(&self.automatic_tax)
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        json!({ "enabled": enabled, "status": null })
    }

    /// Aggregate the stored per-line tax amounts by tax-rate id.
    fn tax_breakdown(&self) -> Vec<Value> {
        let mut totals: Vec<(String, i64, i64, Value)> = Vec::new();
        for line in self.line_items() {
            let Some(amounts) = line.get("tax_amounts").and_then(Value::as_array) else {
                continue;
            };
            for amount in amounts {
                let rate = amount.get("tax_rate").cloned().unwrap_or(Value::Null);
                let rate_id = rate
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let tax = amount.get("amount").and_then(Value::as_i64).unwrap_or(0);
                let taxable = amount
                    .get("taxable_amount")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                match totals.iter_mut().find(|(id, _, _, _)| id == &rate_id) {
                    Some((_, total, taxable_total, _)) => {
                        *total += tax;
                        *taxable_total += taxable;
                    }
                    None => totals.push((rate_id, tax, taxable, rate)),
                }
            }
        }
        totals
            .into_iter()
            .map(|(id, amount, taxable_amount, rate)| {
                json!({
                    "amount": amount,
                    "taxable_amount": taxable_amount,
                    "tax_rate": id,
                    "tax_rate_object": rate,
                })
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        let line_items = parse_json(&Some(self.line_items.clone()));
        let stored_discounts = self.discounts();
        let breakdown_discounts: Vec<Value> = stored_discounts
            .iter()
            .map(|discount| {
                json!({
                    "amount": discount.get("amount").cloned().unwrap_or(json!(0)),
                    "coupon": discount
                        .get("coupon")
                        .and_then(|coupon| coupon.get("id"))
                        .cloned()
                        .unwrap_or(Value::Null),
                })
            })
            .collect();
        let taxes = self.tax_breakdown();
        // Rows created before the total columns existed keep their old total
        // and report zero discount and tax.
        let amount_subtotal = self.amount_subtotal.or(self.amount_total).unwrap_or(0);
        let amount_discount = self.amount_discount.unwrap_or(0);
        let amount_tax = self.amount_tax.unwrap_or(0);
        let amount_total = self.amount_total.unwrap_or(amount_subtotal);
        json!({
            "id": self.id,
            "object": "checkout.session",
            "amount_subtotal": amount_subtotal,
            "amount_total": amount_total,
            "amount_discount": amount_discount,
            "amount_tax": amount_tax,
            "automatic_tax": self.automatic_tax(),
            "cancel_url": self.cancel_url,
            "created": self.created,
            "currency": self.currency,
            "customer": self.customer,
            "customer_account": self.customer_account,
            "customer_details": {
                "address": null,
                "email": self.customer_email,
                "name": null,
                "phone": null,
                "tax_exempt": "unknown",
                "tax_ids": [],
            },
            "customer_email": self.customer_email,
            "discounts": stored_discounts,
            "expires_at": self.expires_at,
            "client_secret": self.client_secret,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "mode": self.mode,
            "payment_intent": self.payment_intent,
            "payment_method_collection": self.payment_method_collection,
            "payment_status": if self.status == "complete" { "paid" } else { "unpaid" },
            "return_url": self.return_url,
            "status": self.status,
            "subscription": self.subscription,
            "success_url": self.success_url,
            "ui_mode": self.ui_mode,
            "total_details": {
                "amount_discount": amount_discount,
                "amount_tax": amount_tax,
                "breakdown": {
                    "discounts": breakdown_discounts,
                    "taxes": taxes,
                },
            },
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
pub struct InvoiceRow {
    pub id: String,
    pub account_id: String,
    pub customer: String,
    pub status: String,
    pub currency: String,
    pub created: i64,
    pub updated: i64,
    pub finalized_at: Option<i64>,
    pub paid_at: Option<i64>,
    pub voided_at: Option<i64>,
    pub due_date: Option<i64>,
    pub amount_due: i64,
    pub amount_paid: i64,
    pub amount_remaining: i64,
    pub subtotal: i64,
    pub tax_total: i64,
    pub total: i64,
    pub lines: String,
    pub customer_tax_ids: String,
    pub description: Option<String>,
    pub metadata: Option<String>,
    pub subscription: Option<String>,
    pub billing_reason: Option<String>,
    pub period_start: Option<i64>,
    pub period_end: Option<i64>,
    pub collection_method: String,
    pub attempt_count: i64,
    pub payment_intent: Option<String>,
    pub test_clock: Option<String>,
    pub subscription_metadata: Option<String>,
}

impl InvoiceRow {
    /// The invoice's lines as a list of line item objects.
    pub fn line_list(&self) -> Vec<Value> {
        parse_json(&Some(self.lines.clone()))
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// The snapshot of the subscription's metadata taken when the invoice was
    /// raised.
    fn subscription_metadata_value(&self) -> Value {
        match parse_json(&self.subscription_metadata) {
            Value::Object(map) => Value::Object(map),
            _ => json!({}),
        }
    }

    /// The invoice's `parent`: subscription details for a subscription invoice,
    /// otherwise null.
    fn parent(&self) -> Value {
        match &self.subscription {
            Some(subscription) => json!({
                "type": "subscription_details",
                "subscription_details": {
                    "metadata": self.subscription_metadata_value(),
                    "subscription": subscription,
                },
            }),
            None => Value::Null,
        }
    }

    /// Tax totals aggregated per applied tax rate across the lines.
    pub fn total_taxes(&self) -> Vec<Value> {
        let mut totals: Vec<(String, i64, i64)> = Vec::new();
        for line in self.line_list() {
            let Some(amounts) = line.get("tax_amounts").and_then(Value::as_array) else {
                continue;
            };
            for amount in amounts {
                let rate = amount.get("tax_rate").cloned().unwrap_or(Value::Null);
                let rate_id = rate
                    .get("id")
                    .and_then(Value::as_str)
                    .or_else(|| rate.as_str())
                    .unwrap_or_default()
                    .to_string();
                let tax = amount.get("amount").and_then(Value::as_i64).unwrap_or(0);
                let taxable = amount
                    .get("taxable_amount")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                match totals.iter_mut().find(|(id, _, _)| id == &rate_id) {
                    Some((_, total, taxable_total)) => {
                        *total += tax;
                        *taxable_total += taxable;
                    }
                    None => totals.push((rate_id, tax, taxable)),
                }
            }
        }
        totals
            .into_iter()
            .map(|(rate, amount, taxable_amount)| {
                json!({
                    "amount": amount,
                    "tax_rate": rate,
                    "taxable_amount": taxable_amount,
                })
            })
            .collect()
    }

    pub fn to_json(&self, base_url: &str) -> Value {
        json!({
            "id": self.id,
            "object": "invoice",
            "account_country": "US",
            "amount_due": self.amount_due,
            "amount_overpaid": 0,
            "amount_paid": self.amount_paid,
            "amount_remaining": self.remaining(),
            "attempt_count": self.attempt_count,
            "attempted": self.attempted(),
            "auto_advance": false,
            "automatic_tax": { "enabled": false, "status": null },
            "billing_reason": self.billing_reason,
            "collection_method": self.collection_method,
            "created": self.created,
            "currency": self.currency,
            "customer": self.customer,
            "customer_tax_ids": parse_json(&Some(self.customer_tax_ids.clone()))
                .as_array()
                .cloned()
                .unwrap_or_default(),
            "description": self.description,
            "due_date": self.due_date,
            "finalized_at": self.finalized_at,
            "hosted_invoice_url": self.hosted_invoice_url(base_url),
            "invoice_pdf": self.invoice_pdf(base_url),
            "lines": list_envelope(
                &format!("/v1/invoices/{}/lines", self.id),
                self.line_list(),
                false,
            ),
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "number": Value::Null,
            "paid": self.status == "paid",
            "paid_at": self.paid_at,
            "parent": self.parent(),
            "payment_intent": self.payment_intent,
            "period_end": self.period_end,
            "period_start": self.period_start,
            "status": self.status,
            "subscription": self.subscription,
            "subtotal": self.subtotal,
            "tax_total": self.tax_total,
            "test_clock": self.test_clock,
            "total": self.total,
            "total_taxes": self.total_taxes(),
            "updated": self.updated,
            "voided_at": self.voided_at,
        })
    }

    /// The URL of the invoice's hosted page.
    ///
    /// Stripe publishes the page once the invoice is finalized, so a draft
    /// invoice has none.
    pub fn hosted_invoice_url(&self, base_url: &str) -> Option<String> {
        if self.status == "draft" {
            return None;
        }
        Some(format!("{base_url}/mock/invoices/{}", self.id))
    }

    /// The URL of the invoice's PDF, which exists once the invoice is
    /// finalized.
    pub fn invoice_pdf(&self, base_url: &str) -> Option<String> {
        if self.status == "draft" {
            return None;
        }
        Some(format!("{base_url}/mock/invoices/{}/pdf", self.id))
    }

    /// The unpaid balance: `amount_due` minus `amount_paid`, never negative.
    pub fn remaining(&self) -> i64 {
        (self.amount_due - self.amount_paid).max(0)
    }

    /// Whether the invoice has been attempted at least once.
    fn attempted(&self) -> bool {
        self.attempt_count > 0 || self.status == "paid" || self.status == "uncollectible"
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
    pub invoice: Option<String>,
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
            "invoice": self.invoice,
            "livemode": false,
            "metadata": parse_json_or_empty(&self.metadata),
            "status": self.status,
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MeterRow {
    pub id: String,
    pub account_id: String,
    pub display_name: String,
    pub event_name: String,
    pub aggregation_formula: String,
    pub customer_payload_key: String,
    pub value_payload_key: String,
    pub event_time_window: Option<String>,
    pub status: String,
    pub deactivated_at: Option<i64>,
    pub created: i64,
    pub updated: i64,
}

impl MeterRow {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "object": "billing.meter",
            "created": self.created,
            "updated": self.updated,
            "display_name": self.display_name,
            "event_name": self.event_name,
            "default_aggregation": { "formula": self.aggregation_formula },
            "customer_mapping": {
                "event_payload_key": self.customer_payload_key,
                "type": "by_id",
            },
            "value_settings": { "event_payload_key": self.value_payload_key },
            "event_time_window": self.event_time_window,
            "livemode": false,
            "status": self.status,
            "status_transitions": { "deactivated_at": self.deactivated_at },
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MeterEventRow {
    pub id: String,
    pub account_id: String,
    pub meter_id: String,
    pub event_name: String,
    pub identifier: String,
    pub customer: Option<String>,
    pub payload: String,
    pub value: f64,
    pub timestamp: i64,
    pub created: i64,
}

impl MeterEventRow {
    /// The v1 `billing.meter_event` object. Stripe does not expose the internal
    /// event id on this object; the client-supplied identifier is the key.
    pub fn to_json(&self) -> Value {
        json!({
            "object": "billing.meter_event",
            "created": self.created,
            "event_name": self.event_name,
            "identifier": self.identifier,
            "livemode": false,
            "payload": parse_json_or_empty(&Some(self.payload.clone())),
            "timestamp": self.timestamp,
        })
    }
}

/// Render an aggregated number without a trailing `.0` when it is whole.
pub fn usage_number(value: f64) -> Value {
    if value.fract() == 0.0 {
        json!(value as i64)
    } else {
        json!(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn customer(tax_ids: &str) -> CustomerRow {
        CustomerRow {
            id: "cus_1".into(),
            account_id: "acct_1".into(),
            email: Some("buyer@example.com".into()),
            name: Some("Buyer".into()),
            currency: Some("usd".into()),
            created: 10,
            tax_ids: Some(tax_ids.into()),
            metadata: Some("{}".into()),
            invoice_settings: None,
            test_clock: None,
        }
    }

    fn checkout(customer_email: Option<&str>) -> CheckoutSessionRow {
        CheckoutSessionRow {
            id: "cs_test_1".into(),
            account_id: "acct_1".into(),
            mode: "payment".into(),
            status: "open".into(),
            created: 10,
            expires_at: 20,
            success_url: None,
            cancel_url: None,
            customer: Some("cus_1".into()),
            customer_account: None,
            customer_email: customer_email.map(str::to_string),
            line_items: "[]".into(),
            payment_intent: None,
            subscription: None,
            amount_subtotal: Some(2000),
            amount_total: Some(1650),
            amount_discount: Some(500),
            amount_tax: Some(150),
            discounts: Some(
                r#"[{"id":"di_1","amount":500,"coupon":{"id":"coupon_1","object":"coupon"}}]"#
                    .into(),
            ),
            currency: Some("usd".into()),
            url: None,
            metadata: Some("{}".into()),
            ui_mode: "hosted_page".into(),
            return_url: None,
            client_secret: None,
            subscription_metadata: None,
            payment_method_collection: "always".into(),
            automatic_tax: Some(r#"{"enabled":false}"#.into()),
        }
    }

    #[test]
    fn checkout_renders_the_customer_email_and_details() {
        let json = checkout(Some("buyer@example.com")).to_json();
        assert_eq!(json["customer_email"], "buyer@example.com");
        assert_eq!(json["customer_details"]["email"], "buyer@example.com");
    }

    #[test]
    fn checkout_without_an_email_renders_null() {
        let json = checkout(None).to_json();
        assert_eq!(json["customer_email"], Value::Null);
        assert_eq!(json["customer_details"]["email"], Value::Null);
    }

    #[test]
    fn checkout_renders_totals_and_breakdowns() {
        let mut row = checkout(None);
        row.line_items = serde_json::to_string(&json!([
            {
                "id": "li_1",
                "tax_rates": [{"id": "txr_1", "percentage": 10}],
                "tax_amounts": [
                    {"amount": 75, "taxable_amount": 750, "tax_rate": {"id": "txr_1", "percentage": 10}}
                ]
            },
            {
                "id": "li_2",
                "tax_rates": [{"id": "txr_1", "percentage": 10}],
                "tax_amounts": [
                    {"amount": 75, "taxable_amount": 750, "tax_rate": {"id": "txr_1", "percentage": 10}}
                ]
            }
        ]))
        .unwrap();
        let json = row.to_json();
        assert_eq!(json["amount_subtotal"], 2000);
        assert_eq!(json["amount_discount"], 500);
        assert_eq!(json["amount_tax"], 150);
        assert_eq!(json["amount_total"], 1650);
        assert_eq!(json["total_details"]["amount_discount"], 500);
        assert_eq!(
            json["total_details"]["breakdown"]["discounts"][0]["coupon"],
            "coupon_1"
        );
        assert_eq!(
            json["total_details"]["breakdown"]["discounts"][0]["amount"],
            500
        );
        let taxes = json["total_details"]["breakdown"]["taxes"]
            .as_array()
            .unwrap();
        assert_eq!(taxes.len(), 1, "tax rates aggregate by id");
        assert_eq!(taxes[0]["tax_rate"], "txr_1");
        assert_eq!(taxes[0]["amount"], 150);
        assert_eq!(taxes[0]["taxable_amount"], 1500);
    }

    #[test]
    fn legacy_checkout_rows_fall_back_to_the_total() {
        let mut row = checkout(None);
        row.amount_subtotal = None;
        row.amount_discount = None;
        row.amount_tax = None;
        row.discounts = None;
        row.amount_total = Some(1000);
        let json = row.to_json();
        assert_eq!(json["amount_subtotal"], 1000);
        assert_eq!(json["amount_total"], 1000);
        assert_eq!(json["amount_discount"], 0);
        assert_eq!(json["amount_tax"], 0);
        assert_eq!(json["discounts"], json!([]));
        assert_eq!(json["total_details"]["breakdown"]["taxes"], json!([]));
    }

    #[test]
    fn checkout_echoes_payment_method_collection_and_automatic_tax() {
        let json = checkout(None).to_json();
        assert_eq!(json["payment_method_collection"], "always");
        assert_eq!(json["automatic_tax"]["enabled"], false);
        assert_eq!(json["automatic_tax"]["status"], Value::Null);

        let mut row = checkout(None);
        row.payment_method_collection = "if_required".into();
        row.automatic_tax = Some(r#"{"enabled":true}"#.into());
        let json = row.to_json();
        assert_eq!(json["payment_method_collection"], "if_required");
        assert_eq!(json["automatic_tax"]["enabled"], true);
        assert_eq!(json["automatic_tax"]["status"], Value::Null);

        // A row from before the columns existed renders the defaults.
        let mut legacy = checkout(None);
        legacy.automatic_tax = None;
        let json = legacy.to_json();
        assert_eq!(json["automatic_tax"]["enabled"], false);
        assert_eq!(json["automatic_tax"]["status"], Value::Null);
    }

    #[test]
    fn subscription_renders_billing_fields_with_null_defaults() {
        let row = SubscriptionRow {
            id: "sub_1".into(),
            account_id: "acct_1".into(),
            customer: Some("cus_1".into()),
            customer_account: None,
            status: "active".into(),
            created: 90,
            current_period_start: Some(100),
            current_period_end: Some(200),
            cancel_at_period_end: 0,
            items: "[]".into(),
            metadata: Some("{}".into()),
            test_clock: None,
            latest_invoice: None,
            default_payment_method: None,
            cancel_at: None,
            canceled_at: None,
            ended_at: None,
            billing_cycle_anchor_config: None,
        };
        let json = row.to_json();
        assert_eq!(json["latest_invoice"], Value::Null);
        assert_eq!(json["default_payment_method"], Value::Null);
        assert_eq!(json["cancel_at"], Value::Null);
        assert_eq!(json["canceled_at"], Value::Null);
        assert_eq!(json["ended_at"], Value::Null);
        assert_eq!(json["billing_cycle_anchor_config"], Value::Null);
    }

    #[test]
    fn billing_portal_session_renders_its_fields() {
        let row = BillingPortalSessionRow {
            id: "bps_1".into(),
            account_id: "acct_1".into(),
            customer: "cus_1".into(),
            return_url: "https://example.com/account".into(),
            url: "http://127.0.0.1:12111/mock/billing-portal/bps_1".into(),
            created: 10,
        };
        let json = row.to_json();
        assert_eq!(json["object"], "billing_portal.session");
        assert_eq!(json["id"], "bps_1");
        assert_eq!(json["customer"], "cus_1");
        assert_eq!(json["return_url"], "https://example.com/account");
        assert_eq!(
            json["url"],
            "http://127.0.0.1:12111/mock/billing-portal/bps_1"
        );
        assert_eq!(json["created"], 10);
    }

    #[test]
    fn customer_renders_tax_ids() {
        let row = customer(
            r#"[{"id":"txi_1","object":"tax_id","type":"eu_vat","value":"DE1"},{"id":"txi_2","object":"tax_id","type":"de_vat","value":"DE2"}]"#,
        );
        let json = row.to_json();
        assert_eq!(json["tax_ids"].as_array().unwrap().len(), 2);
        assert_eq!(json["tax_ids"][0]["id"], "txi_1");
        assert_eq!(json["tax_ids"][0]["type"], "eu_vat");
        assert_eq!(json["tax_ids"][1]["value"], "DE2");
    }

    #[test]
    fn customer_without_tax_ids_renders_empty_list() {
        let json = customer("").to_json();
        assert_eq!(json["tax_ids"], json!({}));
        assert!(customer("").tax_id_list().is_empty());
    }

    #[test]
    fn tax_id_entry_shape() {
        let entry = tax_id_entry("txi_1", "eu_vat", "DE123", 42);
        assert_eq!(entry["id"], "txi_1");
        assert_eq!(entry["object"], "tax_id");
        assert_eq!(entry["type"], "eu_vat");
        assert_eq!(entry["value"], "DE123");
        assert_eq!(entry["created"], 42);
        assert_eq!(entry["livemode"], false);
    }

    /// The base url the hosted invoice url is built from.
    const BASE_URL: &str = "http://127.0.0.1:12111";

    fn invoice(amount_due: i64, amount_paid: i64, status: &str) -> InvoiceRow {
        InvoiceRow {
            id: "in_1".into(),
            account_id: "acct_1".into(),
            customer: "cus_1".into(),
            status: status.into(),
            currency: "usd".into(),
            created: 10,
            updated: 10,
            finalized_at: Some(11),
            paid_at: if status == "paid" { Some(12) } else { None },
            voided_at: None,
            due_date: Some(20),
            amount_due,
            amount_paid,
            amount_remaining: 0,
            subtotal: amount_due,
            tax_total: 0,
            total: amount_due,
            lines: r#"[{"id":"il_1","object":"line_item","amount":1000}]"#.into(),
            customer_tax_ids: "[]".into(),
            description: None,
            metadata: Some("{}".into()),
            subscription: None,
            billing_reason: None,
            period_start: None,
            period_end: None,
            collection_method: "send_invoice".into(),
            attempt_count: 0,
            payment_intent: None,
            test_clock: None,
            subscription_metadata: None,
        }
    }

    #[test]
    fn scoped_objects_render_the_test_clock() {
        let mut customer = customer("[]");
        assert_eq!(customer.to_json()["test_clock"], Value::Null);
        customer.test_clock = Some("clock_1".into());
        assert_eq!(customer.to_json()["test_clock"], "clock_1");

        let mut invoice = invoice(1000, 1000, "paid");
        assert_eq!(invoice.to_json(BASE_URL)["test_clock"], Value::Null);
        invoice.test_clock = Some("clock_1".into());
        assert_eq!(invoice.to_json(BASE_URL)["test_clock"], "clock_1");
    }

    #[test]
    fn test_clock_renders_status_details() {
        let row = TestClockRow {
            id: "clock_1".into(),
            account_id: "acct_1".into(),
            name: Some("Annual renewal".into()),
            frozen_time: 1_577_836_800,
            status: "ready".into(),
            created: 10,
            deletes_after: 30,
        };
        let json = row.to_json();
        assert_eq!(json["object"], "test_helpers.test_clock");
        assert_eq!(json["frozen_time"], 1_577_836_800);
        assert_eq!(json["status"], "ready");
        assert_eq!(json["status_details"]["advancing"], Value::Null);

        let advancing = row.render("advancing", json!({"target_frozen_time": 1_580_428_800}));
        assert_eq!(advancing["status"], "advancing");
        assert_eq!(
            advancing["status_details"]["advancing"]["target_frozen_time"],
            1_580_428_800
        );
    }

    #[test]
    fn paid_invoice_has_no_remaining_balance() {
        let json = invoice(1100, 1100, "paid").to_json(BASE_URL);
        assert_eq!(json["amount_remaining"], 0);
        assert_eq!(json["amount_paid"], 1100);
        assert_eq!(json["paid"], true);
        assert_eq!(json["status"], "paid");
    }

    #[test]
    fn partially_paid_invoice_keeps_the_balance() {
        let json = invoice(1100, 400, "open").to_json(BASE_URL);
        assert_eq!(json["amount_remaining"], 700);
        assert_eq!(json["paid"], false);
    }

    #[test]
    fn invoice_renders_lines_as_a_list_envelope() {
        let json = invoice(1000, 0, "draft").to_json(BASE_URL);
        assert_eq!(json["lines"]["object"], "list");
        assert_eq!(json["lines"]["data"][0]["id"], "il_1");
        assert_eq!(json["lines"]["url"], "/v1/invoices/in_1/lines");
    }

    fn coupon(
        times_redeemed: i64,
        max_redemptions: Option<i64>,
        redeem_by: Option<i64>,
    ) -> CouponRow {
        CouponRow {
            id: "coupon_1".into(),
            account_id: "acct_1".into(),
            percent_off: Some(25.0),
            amount_off: None,
            currency: Some("usd".into()),
            duration: "once".into(),
            duration_in_months: None,
            max_redemptions,
            times_redeemed,
            redeem_by,
            name: Some("Quarter off".into()),
            applies_to: Some(r#"{"products":["prod_1"]}"#.into()),
            metadata: Some("{}".into()),
            created: 10,
        }
    }

    #[test]
    fn valid_percent_coupon_renders_its_fields() {
        let json = coupon(0, None, None).to_json();
        assert_eq!(json["object"], "coupon");
        assert_eq!(json["percent_off"], 25);
        assert_eq!(json["amount_off"], Value::Null);
        assert_eq!(json["valid"], true);
        assert_eq!(json["times_redeemed"], 0);
        assert_eq!(json["applies_to"]["products"][0], "prod_1");
    }

    #[test]
    fn expired_coupon_is_not_valid() {
        let json = coupon(0, None, Some(5)).to_json();
        assert_eq!(json["valid"], false);
    }

    #[test]
    fn fully_redeemed_coupon_is_not_valid() {
        let json = coupon(3, Some(3), None).to_json();
        assert_eq!(json["valid"], false);
        assert!(!coupon(3, Some(3), None).is_valid(0));
        assert!(coupon(2, Some(3), None).is_valid(0));
    }

    #[test]
    fn subscription_items_keep_or_inherit_their_periods() {
        let new_item = json!({
            "id": "si_1",
            "object": "subscription_item",
            "current_period_start": 100,
            "current_period_end": 200,
            "subscription": "sub_1",
            "price": {"id": "price_1"},
            "quantity": 2
        });
        let row = SubscriptionRow {
            id: "sub_1".into(),
            account_id: "acct_1".into(),
            customer: Some("cus_1".into()),
            customer_account: None,
            status: "active".into(),
            created: 90,
            current_period_start: Some(100),
            current_period_end: Some(200),
            cancel_at_period_end: 0,
            items: serde_json::to_string(&json!([new_item])).unwrap(),
            metadata: Some("{}".into()),
            test_clock: None,
            latest_invoice: None,
            default_payment_method: None,
            cancel_at: None,
            canceled_at: None,
            ended_at: None,
            billing_cycle_anchor_config: None,
        };
        let json = row.to_json();
        let item = &json["items"]["data"][0];
        assert_eq!(item["subscription"], "sub_1");
        assert_eq!(item["current_period_start"], 100);
        assert_eq!(item["current_period_end"], 200);

        // An item stored by an older build gains the fields from the parent.
        let legacy = SubscriptionRow {
            items: r#"[{"id":"si_old","object":"subscription_item","price":{"id":"price_1"},"quantity":1}]"#
                .into(),
            ..row
        };
        let json = legacy.to_json();
        let item = &json["items"]["data"][0];
        assert_eq!(item["subscription"], "sub_1");
        assert_eq!(item["current_period_start"], 100);
        assert_eq!(item["current_period_end"], 200);
    }

    #[test]
    fn tax_rate_renders_its_flags_and_percentage() {
        let row = TaxRateRow {
            id: "txr_1".into(),
            account_id: "acct_1".into(),
            active: 1,
            display_name: "VAT".into(),
            percentage: 20.0,
            inclusive: 0,
            country: Some("DE".into()),
            state: None,
            jurisdiction: Some("DE".into()),
            description: Some("German VAT".into()),
            tax_type: Some("vat".into()),
            metadata: Some("{}".into()),
            created: 10,
        };
        let json = row.to_json();
        assert_eq!(json["object"], "tax_rate");
        assert_eq!(json["display_name"], "VAT");
        assert_eq!(json["percentage"], 20);
        assert_eq!(json["inclusive"], false);
        assert_eq!(json["active"], true);
        assert_eq!(json["jurisdiction"], "DE");
    }

    #[test]
    fn price_renders_its_lookup_key_or_null() {
        let mut row = PriceRow {
            id: "price_1".into(),
            account_id: "acct_1".into(),
            product_id: "prod_1".into(),
            currency: "usd".into(),
            unit_amount: Some(1000),
            price_type: "recurring".into(),
            recurring_interval: Some("month".into()),
            recurring_interval_count: Some(1),
            nickname: None,
            active: 1,
            created: 10,
            metadata: Some("{}".into()),
            lookup_key: None,
            billing_scheme: "per_unit".into(),
            tiers: None,
            tiers_mode: None,
            usage_type: None,
            meter_id: None,
        };
        assert_eq!(row.to_json()["lookup_key"], Value::Null);
        row.lookup_key = Some("pro_monthly".into());
        assert_eq!(row.to_json()["lookup_key"], "pro_monthly");
    }

    #[test]
    fn payment_intent_renders_its_invoice() {
        let row = PaymentIntentRow {
            id: "pi_1".into(),
            account_id: "acct_1".into(),
            amount: Some(1000),
            currency: Some("usd".into()),
            status: "requires_payment_method".into(),
            customer: Some("cus_1".into()),
            invoice: Some("in_1".into()),
            created: 10,
            checkout_session: None,
            metadata: Some("{}".into()),
        };
        let json = row.to_json();
        assert_eq!(json["invoice"], "in_1");
        assert_eq!(json["object"], "payment_intent");
        assert_eq!(json["amount"], 1000);
    }
}
