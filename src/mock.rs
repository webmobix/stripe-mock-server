use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde_json::{Map, Value, json};
use sqlx::SqlitePool;

use crate::AppState;
use crate::api::v1;
use crate::events;
use crate::models::{AccountRow, CheckoutSessionRow, InvoiceRow};
use crate::time;

/// Well-known checkout session id that is always available so the mock checkout
/// page can be opened and exercised without creating a session through the API.
pub const STATIC_CHECKOUT_ID: &str = "cs_test_static";

/// The shared CDN-loaded HTML shell. No client-side framework bundle is used.
pub fn shell(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en" data-theme="light">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{title}</title>
  <script src="https://cdn.tailwindcss.com"></script>
  <link href="https://cdn.jsdelivr.net/npm/daisyui@4.12.14/dist/full.min.css" rel="stylesheet" type="text/css" />
</head>
<body class="min-h-screen bg-base-200">
  <div class="mx-auto max-w-3xl p-6">
    <header class="mb-6 flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-bold">Stripe Mock</h1>
        <p class="text-sm opacity-70">{title}</p>
      </div>
    </header>
    <main>{body}</main>
  </div>
</body>
</html>"#
    )
}

/// A shared simulation panel with the controls applicable to each page.
///
/// `cancel_href` supplies the checkout cancel control target; onboarding passes
/// `None`.
pub fn simulation_panel(kind: PanelKind, cancel_href: Option<&str>) -> String {
    let mut controls = String::new();
    match kind {
        PanelKind::Onboarding => {
            controls.push_str(
                r#"<div class="form-control">
  <label class="label"><span class="label-text">Simulate identity verification (KYC)</span></label>
  <div class="join">
    <button class="btn join-item btn-success" name="kyc" value="pass">KYC pass</button>
    <button class="btn join-item btn-error" name="kyc" value="fail">KYC fail</button>
  </div>
</div>
<div class="form-control">
  <label class="label"><span class="label-text">Payout IBAN</span></label>
  <select class="select select-bordered" name="iban">
    <option value="DE89370400440532013000">DE89 ... 3000</option>
    <option value="GB29NWBK60161331926819">GB29 ... 6819</option>
    <option value="FR1420041010050500013M02606">FR14 ... 2606</option>
  </select>
</div>"#,
            );
        }
        PanelKind::Checkout => {
            let cancel = cancel_href
                .map(|href| {
                    format!(r#"<a id="payment-cancel" class="btn" href="{href}">Cancel</a>"#)
                })
                .unwrap_or_default();
            controls.push_str(&format!(
                r#"<div class="form-control">
  <label class="label"><span class="label-text">Simulate payment</span></label>
  <div class="join">
    <button id="payment-success" class="btn join-item btn-success" name="card" value="success">Successful</button>
    <button id="payment-decline" class="btn join-item btn-error" name="card" value="fail">Declined</button>
    {cancel}
  </div>
</div>
<div class="form-control">
  <label class="label"><span class="label-text">Alternative payment method</span></label>
  <select class="select select-bordered" name="payment_method">
    <option value="card">Card</option>
    <option value="iban">SEPA debit</option>
  </select>
</div>"#
            ));
        }
    }
    format!(
        r#"<section class="card bg-base-100 shadow mt-6">
  <div class="card-body">
    {controls}
  </div>
</section>"#
    )
}

#[derive(Clone, Copy)]
pub enum PanelKind {
    Onboarding,
    Checkout,
}

fn page_error(message: &str) -> Response {
    let body = format!(r#"<div class="alert alert-error">{message}</div>"#);
    (
        axum::http::StatusCode::NOT_FOUND,
        Html(shell("Not found", &body)),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Onboarding
// ---------------------------------------------------------------------------

/// Emit a thin event about an account, owned by the platform. Used by
/// onboarding so an existing account can drive v2 event consumers.
async fn emit_account_thin(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: &str,
    account_id: &str,
    event_type: &str,
    data: Value,
) -> Result<String, sqlx::Error> {
    let envelope = events::thin_envelope(
        events::account_related_object(account_id),
        data,
        Value::Null,
    );
    events::emit_thin(tx, owner, Some(account_id), event_type, envelope, None).await
}

pub async fn onboarding_page(State(st): State<AppState>, Path(token): Path<String>) -> Response {
    let link = match load_link(&st.pool, &token).await {
        Ok(Some(link)) => link,
        _ => return page_error("This onboarding link is invalid."),
    };
    if link.used != 0 || link.expires_at < time::now_epoch() {
        if let Some(refresh) = link.refresh_url {
            return Redirect::to(&refresh).into_response();
        }
        return page_error("This onboarding link has expired.");
    }
    let account: AccountRow = match sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&link.account_id)
        .fetch_one(&st.pool)
        .await
    {
        Ok(account) => account,
        Err(_) => return page_error("The account for this link no longer exists."),
    };
    let body = format!(
        r#"<div class="card bg-base-100 shadow">
  <div class="card-body">
    <h2 class="card-title">Onboarding for {account_id}</h2>
    <form method="post" action="/mock/connect/{token}">
      <div class="form-control mb-4">
        <label class="label"><span class="label-text">Business name</span></label>
        <input class="input input-bordered" name="business_name" value="{business_name}" />
      </div>
      {panel}
    </form>
  </div>
</div>"#,
        account_id = crate::util::html_escape(&account.id),
        token = crate::util::html_escape(&token),
        business_name = "Mock Business",
        panel = simulation_panel(PanelKind::Onboarding, None),
    );
    Html(shell("Connected account onboarding", &body)).into_response()
}

pub async fn onboarding_submit(
    State(st): State<AppState>,
    Path(token): Path<String>,
    body: String,
) -> Response {
    let params = crate::form::parse_form(&body);
    let kyc = params.get("kyc").and_then(|v| v.as_str()).unwrap_or("pass");
    let link = match load_link(&st.pool, &token).await {
        Ok(Some(link)) if link.used == 0 && link.expires_at >= time::now_epoch() => link,
        Ok(Some(link)) => {
            return match link.refresh_url {
                Some(refresh) => Redirect::to(&refresh).into_response(),
                None => page_error("This onboarding link has expired."),
            };
        }
        _ => return page_error("This onboarding link is invalid."),
    };

    let is_main: i64 = sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
        .bind(&link.account_id)
        .fetch_one(&st.pool)
        .await
        .unwrap_or(1);
    let is_connected = is_main == 0;

    let result = {
        let mut tx = match st.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => return internal_error(e),
        };
        let now = time::now_epoch();
        let mut changed_capabilities: Vec<String>;
        if kyc == "pass" {
            // Activate exactly the capabilities this account requested.
            let current: Option<String> =
                match sqlx::query_scalar("SELECT capabilities FROM accounts WHERE id = ?1")
                    .bind(&link.account_id)
                    .fetch_one(&mut *tx)
                    .await
                {
                    Ok(value) => value,
                    Err(e) => return internal_error(e),
                };
            let mut capabilities = match crate::models::parse_json(&current) {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            changed_capabilities = capabilities
                .iter()
                .filter(|(_, status)| status.as_str() != Some("active"))
                .map(|(name, _)| name.clone())
                .collect();
            for status in capabilities.values_mut() {
                *status = json!("active");
            }
            if capabilities.is_empty() {
                capabilities.insert("card_payments".to_string(), json!("active"));
                changed_capabilities.push("card_payments".to_string());
            }
            if let Err(e) = sqlx::query(
                "UPDATE accounts SET details_submitted = 1, charges_enabled = 1, payouts_enabled = 1, capabilities = ?1, updated = ?2 WHERE id = ?3",
            )
            .bind(Value::Object(capabilities).to_string())
            .bind(now)
            .bind(&link.account_id)
            .execute(&mut *tx)
            .await
            {
                return internal_error(e);
            }
        } else {
            let requirements = json!({
                "alternatives": [],
                "current_deadline": null,
                "currently_due": ["business_profile.url", "individual.id_number"],
                "disabled_reason": "requirements.past_due",
                "eventually_due": ["business_profile.url", "individual.id_number"],
                "past_due": [],
                "pending_verification": [],
            });
            // A failed review also takes the account out of service.
            let current: Option<String> =
                match sqlx::query_scalar("SELECT capabilities FROM accounts WHERE id = ?1")
                    .bind(&link.account_id)
                    .fetch_one(&mut *tx)
                    .await
                {
                    Ok(value) => value,
                    Err(e) => return internal_error(e),
                };
            let mut capabilities = match crate::models::parse_json(&current) {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            changed_capabilities = capabilities
                .iter()
                .filter(|(_, status)| status.as_str() == Some("active"))
                .map(|(name, _)| name.clone())
                .collect();
            for status in capabilities.values_mut() {
                if status.as_str() == Some("active") {
                    *status = json!("inactive");
                }
            }
            if let Err(e) = sqlx::query("UPDATE accounts SET details_submitted = 0, charges_enabled = 0, payouts_enabled = 0, capabilities = ?1, requirements = ?2, updated = ?3 WHERE id = ?4")
                .bind(Value::Object(capabilities).to_string())
                .bind(requirements.to_string())
                .bind(now)
                .bind(&link.account_id)
                .execute(&mut *tx)
                .await
            {
                return internal_error(e);
            }
        }
        let _ = sqlx::query("UPDATE account_links SET used = 1 WHERE token = ?1")
            .bind(&token)
            .execute(&mut *tx)
            .await;

        let account: AccountRow = match sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
            .bind(&link.account_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(account) => account,
            Err(e) => return internal_error(e),
        };
        let mut event_ids = vec![match events::emit(
            &mut tx,
            &link.account_id,
            is_connected,
            "account.updated",
            account.to_v1(),
        )
        .await
        {
            Ok(id) => id,
            Err(e) => return internal_error(e),
        }];

        // Thin v1-family event (`v1.account.updated`). Like the snapshot event
        // above it uses the connected scope; the v2 events below use @self.
        let v1_thin_envelope = events::thin_envelope(
            events::account_v1_related_object(&account.id),
            Value::Null,
            Value::Null,
        );
        match events::emit_thin(
            &mut tx,
            &account.id,
            if is_connected {
                Some(account.id.as_str())
            } else {
                None
            },
            "v1.account.updated",
            v1_thin_envelope,
            None,
        )
        .await
        {
            Ok(id) => event_ids.push(id),
            Err(e) => return internal_error(e),
        }

        // Emit v2 thin account events owned by the platform, so an existing
        // account can drive the same consumers as v2 account creation.
        let owner: Option<String> =
            match sqlx::query_scalar("SELECT id FROM accounts WHERE is_main = 1 LIMIT 1")
                .fetch_optional(&mut *tx)
                .await
            {
                Ok(owner) => owner,
                Err(e) => return internal_error(e),
            };
        if let Some(owner) = owner {
            let config_value = crate::models::parse_json(&account.configuration);
            for capability in &changed_capabilities {
                let section = events::capability_section(&config_value, capability);
                let event_type =
                    format!("v2.core.account[configuration.{section}].capability_status_updated");
                match emit_account_thin(
                    &mut tx,
                    &owner,
                    &account.id,
                    &event_type,
                    json!({ "updated_capability": capability }),
                )
                .await
                {
                    Ok(id) => event_ids.push(id),
                    Err(e) => return internal_error(e),
                }
            }
            if kyc != "pass" {
                match emit_account_thin(
                    &mut tx,
                    &owner,
                    &account.id,
                    "v2.core.account[requirements].updated",
                    json!({}),
                )
                .await
                {
                    Ok(id) => event_ids.push(id),
                    Err(e) => return internal_error(e),
                }
            }
            match emit_account_thin(
                &mut tx,
                &owner,
                &account.id,
                "v2.core.account.updated",
                Value::Null,
            )
            .await
            {
                Ok(id) => event_ids.push(id),
                Err(e) => return internal_error(e),
            }
        }

        if let Err(e) = tx.commit().await {
            return internal_error(e);
        }
        event_ids
    };
    for event_id in &result {
        let _ = st.dispatcher.dispatch_event(event_id).await;
    }

    let redirect = if kyc == "pass" {
        link.return_url.clone().or(link.refresh_url.clone())
    } else {
        link.refresh_url.clone().or(link.return_url.clone())
    };
    match redirect {
        Some(url) => Redirect::to(&url).into_response(),
        None => Html(shell(
            "Onboarding complete",
            r#"<div class="alert alert-success">Onboarding step recorded.</div>"#,
        ))
        .into_response(),
    }
}

struct LinkRow {
    account_id: String,
    refresh_url: Option<String>,
    return_url: Option<String>,
    used: i64,
    expires_at: i64,
}

async fn load_link(pool: &SqlitePool, token: &str) -> Result<Option<LinkRow>, sqlx::Error> {
    let row: Option<(String, Option<String>, Option<String>, i64, i64)> =
        sqlx::query_as("SELECT account_id, refresh_url, return_url, used, expires_at FROM account_links WHERE token = ?1")
            .bind(token)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(
        |(account_id, refresh_url, return_url, used, expires_at)| LinkRow {
            account_id,
            refresh_url,
            return_url,
            used,
            expires_at,
        },
    ))
}

// ---------------------------------------------------------------------------
// Checkout
// ---------------------------------------------------------------------------

pub async fn checkout_page(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let session = if id == STATIC_CHECKOUT_ID {
        match static_session(&st).await {
            Ok(session) => session,
            Err(e) => return internal_error(e),
        }
    } else {
        match load_session(&st.pool, &id).await {
            Ok(Some(session)) => session,
            _ => return page_error("This checkout session is invalid."),
        }
    };
    if session.status == "complete" {
        return redirect_success(&session);
    }
    if session.status == "expired" {
        return redirect_cancel(&session);
    }
    let total = session.amount_total.unwrap_or(0);
    let currency = session
        .currency
        .clone()
        .unwrap_or_else(|| "usd".to_string());
    let escaped_id = crate::util::html_escape(&id);
    let cancel_href = format!("/mock/checkout/{escaped_id}/cancel");
    let body = format!(
        r#"<div class="card bg-base-100 shadow">
  <div class="card-body">
    <h2 class="card-title">Checkout session {id}</h2>
    <p class="text-sm opacity-70">Mode: {mode}</p>
    <div class="stats shadow my-4">
      <div class="stat">
        <div class="stat-title">Amount</div>
        <div class="stat-value">{amount} {currency}</div>
        <div class="stat-desc">{count} line item(s)</div>
      </div>
    </div>
    <form method="post" action="/mock/checkout/{id}">
      {panel}
    </form>
  </div>
</div>"#,
        id = escaped_id,
        mode = crate::util::html_escape(&session.mode),
        amount = crate::util::html_escape(&format_amount(total, &currency)),
        count = session.line_items().len(),
        panel = simulation_panel(PanelKind::Checkout, Some(&cancel_href)),
    );
    Html(shell("Checkout", &body)).into_response()
}

pub async fn checkout_submit(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let params = crate::form::parse_form(&body);
    let card = params
        .get("card")
        .and_then(|v| v.as_str())
        .unwrap_or("success");
    if card != "success" {
        let body = r#"<div class="alert alert-error">The simulated card was declined. Choose the successful card simulation to complete the session.</div>"#;
        return Html(shell("Payment declined", body)).into_response();
    }
    match v1::complete_checkout_session(&st, &id).await {
        Ok(session) => redirect_success(&session),
        Err(err) => err.into_response(),
    }
}

pub async fn checkout_cancel(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    match load_session(&st.pool, &id).await {
        Ok(Some(session)) => redirect_cancel(&session),
        _ => page_error("This checkout session is invalid."),
    }
}

/// Load the static demo session, re-seeding it fresh when it is missing or no
/// longer open so the well-known URL is always testable.
async fn static_session(st: &AppState) -> Result<CheckoutSessionRow, sqlx::Error> {
    if let Some(session) = load_session(&st.pool, STATIC_CHECKOUT_ID).await?
        && session.status == "open"
        && session.expires_at >= time::now_epoch()
    {
        return Ok(session);
    }
    let account_id: String =
        sqlx::query_scalar("SELECT id FROM accounts WHERE is_main = 1 LIMIT 1")
            .fetch_one(&st.pool)
            .await?;
    crate::seed::seed_static_checkout_session(&st.pool, &account_id, &st.config.base_url()).await?;
    load_session(&st.pool, STATIC_CHECKOUT_ID)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

fn redirect_success(session: &CheckoutSessionRow) -> Response {
    match &session.success_url {
        Some(url) => {
            Redirect::to(&url.replace("{CHECKOUT_SESSION_ID}", &session.id)).into_response()
        }
        None => Html(shell(
            "Checkout complete",
            r#"<div class="alert alert-success">Checkout completed successfully.</div>"#,
        ))
        .into_response(),
    }
}

fn redirect_cancel(session: &CheckoutSessionRow) -> Response {
    match &session.cancel_url {
        Some(url) => Redirect::to(url).into_response(),
        None => Html(shell(
            "Checkout canceled",
            r#"<div class="alert alert-info">Checkout was canceled.</div>"#,
        ))
        .into_response(),
    }
}

async fn load_session(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<CheckoutSessionRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

fn internal_error(err: impl std::fmt::Display) -> Response {
    tracing::error!("mock page handler failed, returning 500: {err}");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Html(shell("Error", "<div class=\"alert\">Internal error</div>")),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Hosted invoice
// ---------------------------------------------------------------------------

/// The hosted page of an invoice.
///
/// Stripe publishes this page once the invoice is finalized; the mock serves
/// the same information, so an application that sends its customer to the
/// `hosted_invoice_url` lands on a page that describes the invoice.
pub async fn invoice_page(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let invoice = match load_invoice(&st.pool, &id).await {
        Ok(Some(invoice)) => invoice,
        _ => return page_error("This invoice does not exist."),
    };
    let currency = invoice.currency.clone();
    let rows: String = invoice
        .line_list()
        .iter()
        .map(|line| {
            let description = line
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("Item");
            let amount = line
                .get("amount")
                .and_then(|value| value.as_i64())
                .unwrap_or(0);
            format!(
                r#"<tr><td>{description}</td><td class="text-right">{amount}</td></tr>"#,
                description = crate::util::html_escape(description),
                amount = crate::util::html_escape(&format_amount(amount, &currency)),
            )
        })
        .collect();
    let pdf_href = format!(
        "/mock/invoices/{}/pdf",
        crate::util::html_escape(&invoice.id)
    );
    let body = format!(
        r#"<div class="card bg-base-100 shadow">
  <div class="card-body">
    <h2 class="card-title">Invoice {id}</h2>
    <div class="stats shadow my-4">
      <div class="stat">
        <div class="stat-title">Amount due</div>
        <div class="stat-value">{amount_due}</div>
        <div class="stat-desc">Status: {status}</div>
      </div>
    </div>
    <table class="table">
      <tbody>{rows}</tbody>
      <tfoot>
        <tr><td>Subtotal</td><td class="text-right">{subtotal}</td></tr>
        <tr><td>Tax</td><td class="text-right">{tax_total}</td></tr>
        <tr><td>Total</td><td class="text-right">{total}</td></tr>
      </tfoot>
    </table>
    <a class="btn btn-primary" href="{pdf_href}">Download PDF</a>
  </div>
</div>"#,
        id = crate::util::html_escape(&invoice.id),
        amount_due = crate::util::html_escape(&format_amount(invoice.amount_due, &currency)),
        status = crate::util::html_escape(&invoice.status),
        rows = rows,
        subtotal = crate::util::html_escape(&format_amount(invoice.subtotal, &currency)),
        tax_total = crate::util::html_escape(&format_amount(invoice.tax_total, &currency)),
        total = crate::util::html_escape(&format_amount(invoice.total, &currency)),
        pdf_href = pdf_href,
    );
    Html(shell("Invoice", &body)).into_response()
}

/// The PDF of an invoice.
///
/// Stripe links a PDF of every finalized invoice; the mock serves a minimal
/// one-page document with the invoice's summary.
pub async fn invoice_pdf_page(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let invoice = match load_invoice(&st.pool, &id).await {
        Ok(Some(invoice)) => invoice,
        _ => return page_error("This invoice does not exist."),
    };
    let pdf = invoice_pdf_bytes(&invoice);
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/pdf")],
        pdf,
    )
        .into_response()
}

/// Load an invoice by id, whatever account owns it.
async fn load_invoice(pool: &SqlitePool, id: &str) -> Result<Option<InvoiceRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Render a minimal one-page PDF with the invoice's summary.
///
/// The document is written by hand: a catalog, a page, a font, and the
/// content stream holding the text.
fn invoice_pdf_bytes(invoice: &InvoiceRow) -> Vec<u8> {
    let currency = invoice.currency.clone();
    let lines = [
        format!("Invoice {}", invoice.id),
        format!("Status: {}", invoice.status),
        format!("Customer: {}", invoice.customer),
        format!("Subtotal: {}", format_amount(invoice.subtotal, &currency)),
        format!("Tax: {}", format_amount(invoice.tax_total, &currency)),
        format!("Total: {}", format_amount(invoice.total, &currency)),
        format!(
            "Amount due: {}",
            format_amount(invoice.amount_due, &currency)
        ),
    ];
    let mut content = String::from("BT /F1 12 Tf 50 780 Td 18 TL\n");
    for line in lines {
        content.push_str(&format!("({}) Tj T*\n", escape_pdf_text(&line)));
    }
    content.push_str("ET\n");

    let mut document = String::from("%PDF-1.4\n");
    let mut offsets: Vec<usize> = Vec::new();
    for object in [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ] {
        offsets.push(document.len());
        document.push_str(&format!(
            "{} 0 obj\n{object}\nendobj\n",
            offsets.len()
        ));
    }
    let xref = document.len();
    document.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        offsets.len() + 1
    ));
    for offset in &offsets {
        document.push_str(&format!("{offset:010} 00000 n \n"));
    }
    document.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1
    ));
    document.into_bytes()
}

/// Escape the characters that end a PDF text string.
fn escape_pdf_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '(' | ')' | '\\' => {
                escaped.push('\\');
                escaped.push(character);
            }
            character if character.is_ascii() => escaped.push(character),
            // Anything outside ASCII is written as a placeholder, so the
            // document stays a valid single-byte PDF.
            _ => escaped.push('?'),
        }
    }
    escaped
}

/// Format a minor-unit amount using the currency's number of decimals.
fn format_amount(amount: i64, currency: &str) -> String {
    let decimals = match currency.to_ascii_lowercase().as_str() {
        "jpy" | "krw" | "vnd" | "clp" | "pyg" | "ugx" | "xaf" | "xof" | "xpf" | "bif" | "djf"
        | "gnf" | "kmf" | "rwf" | "vuv" => 0,
        "bhd" | "jod" | "kwd" | "omr" | "tnd" => 3,
        _ => 2,
    };
    if decimals == 0 {
        return amount.to_string();
    }
    let divisor = 10i64.pow(decimals);
    let sign = if amount < 0 { "-" } else { "" };
    let abs = amount.unsigned_abs();
    format!(
        "{sign}{}.{:0width$}",
        abs / divisor as u64,
        abs % divisor as u64,
        width = decimals as usize
    )
}
