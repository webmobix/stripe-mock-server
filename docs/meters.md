# Billing meters

Billing meters collect usage events and aggregate them into the usage a metered
price bills. The mock stores meter events in SQLite, so event summaries and
renewal invoices reflect events reported earlier in the same test.

## Create and manage a meter

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

METER=$(curl -s -X POST $BASE/v1/billing/meters \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'display_name=API calls' \
  --data-urlencode 'event_name=api_call' \
  --data-urlencode 'default_aggregation[formula]=sum')
MID=$(echo "$METER" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
```

A meter has a `mtr_` id and reports:

```json
{
  "id": "mtr_...",
  "object": "billing.meter",
  "display_name": "API calls",
  "event_name": "api_call",
  "default_aggregation": { "formula": "sum" },
  "customer_mapping": { "event_payload_key": "stripe_customer_id", "type": "by_id" },
  "value_settings": { "event_payload_key": "value" },
  "status": "active",
  "status_transitions": { "deactivated_at": null },
  "created": 1760000000,
  "updated": 1760000000,
  "livemode": false
}
```

`default_aggregation.formula` is `sum`, `count`, or `last`. The customer mapping
and value key default to `stripe_customer_id` and `value`; both can be
overridden. Only one active meter may use an `event_name`.

```bash
curl -s "$BASE/v1/billing/meters?status=active" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/billing/meters/$MID" -H "Authorization: Bearer $KEY"
curl -s -X POST "$BASE/v1/billing/meters/$MID" \
  -H "Authorization: Bearer $KEY" -d 'display_name=API requests'
curl -s -X POST "$BASE/v1/billing/meters/$MID/deactivate" -H "Authorization: Bearer $KEY"
curl -s -X POST "$BASE/v1/billing/meters/$MID/reactivate" -H "Authorization: Bearer $KEY"
```

## Report meter events

```bash
curl -s -X POST $BASE/v1/billing/meter_events \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'event_name=api_call' \
  --data-urlencode 'identifier=request-0001' \
  --data-urlencode 'payload[stripe_customer_id]=cus_...' \
  --data-urlencode 'payload[value]=10'
```

The response is a `billing.meter_event` with `event_name`, `identifier`,
`timestamp`, and the string-valued `payload`. `timestamp` defaults to now and
must fall within the past 35 days or five minutes into the future.

`identifier` makes reporting safe to retry. A duplicate identifier is rejected
with HTTP 400 and Stripe's error code:

```json
{
  "error": {
    "type": "invalid_request_error",
    "code": "duplicate_meter_event",
    "message": "A meter event with identifier request-0001 has already been submitted."
  }
}
```

The event must name an **active** meter. An unknown `event_name` is rejected with
the code `no_meter`.

## Event summaries

```bash
CUSTOMER=$(curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" -d 'email=usage@example.com')
CID=$(echo "$CUSTOMER" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

START=$(( ( $(date +%s) / 3600) * 3600 - 3600 ))
END=$(( START + 7200 ))
curl -s "$BASE/v1/billing/meters/$MID/event_summaries?customer=$CID&start_time=$START&end_time=$END&value_grouping_window=hour" \
  -H "Authorization: Bearer $KEY"
```

`customer`, `start_time`, and `end_time` are required. Times are epoch seconds
aligned to minute boundaries; `value_grouping_window=hour` also requires hour
alignment, and `day` requires UTC day alignment. Without a grouping window a
single summary covers the whole range. Summaries are newest-first:

```json
{
  "object": "list",
  "data": [
    {
      "id": "mtrusg_...",
      "object": "billing.meter_event_summary",
      "aggregated_value": 15,
      "start_time": 1760000400,
      "end_time": 1760004000,
      "meter": "mtr_...",
      "livemode": false
    }
  ],
  "has_more": false,
  "url": "/v1/billing/meters/mtr_.../event_summaries"
}
```

## Metered prices

A metered price points at a meter and never takes a quantity:

```bash
curl -s -X POST $BASE/v1/prices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "product=$PRODUCT_ID" \
  --data-urlencode 'currency=usd' \
  --data-urlencode 'recurring[interval]=month' \
  --data-urlencode 'recurring[usage_type]=metered' \
  --data-urlencode "recurring[meter]=$MID" \
  --data-urlencode 'unit_amount=2'
```

Creating or updating a subscription item, or creating a checkout line or
invoice preview override, for a metered price with a `quantity` is rejected with
an `invalid_request_error`; omit the quantity. The subscription item reports
`quantity: null`. Usage is aggregated over the period that ends when the
subscription advances (see [`billing.md`](billing.md#end-a-billing-period)) and
is billed on that renewal invoice. `invoices.createPreview` reports usage
accrued in the current period (see
[`invoicing.md`](invoicing.md#preview-the-upcoming-invoice)).
