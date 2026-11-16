# Test clocks

A test clock freezes the time Stripe uses for the customers attached to it.
Create a customer on a clock, create subscriptions for that customer, then
advance the clock once to drive every renewal, invoice, and cancellation
together — deterministically, without waiting for wall-clock time.

The mock implements the Stripe test-clock resource at
`/v1/test_helpers/test_clocks`, including frozen timestamps on the attached
customer, its subscriptions, and their invoices.

## Object shape

A clock is a `test_helpers.test_clock` with a `clock_` id:

```json
{
  "id": "clock_...",
  "object": "test_helpers.test_clock",
  "created": 1577836800,
  "deletes_after": 1580428800,
  "frozen_time": 1577836800,
  "livemode": false,
  "name": "Annual renewal",
  "status": "ready",
  "status_details": {
    "advancing": null,
    "internal_failure": null
  }
}
```

`frozen_time` is the simulated time in Unix seconds. `created` and
`deletes_after` are wall-clock timestamps: the mock models Stripe's 30-day
cleanup window but never deletes a clock automatically. While an advance is in
flight the response reports `status: "advancing"` with
`status_details.advancing.target_frozen_time`; the committed clock is `ready`.

## Create, retrieve, and list

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

# Create a clock frozen at 2020-01-01T00:00:00Z.
CLOCK=$(curl -s -X POST $BASE/v1/test_helpers/test_clocks \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'frozen_time=1577836800' \
  --data-urlencode 'name=Annual renewal')
CLOCK_ID=$(echo "$CLOCK" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

# Retrieve and list (newest first).
curl -s "$BASE/v1/test_helpers/test_clocks/$CLOCK_ID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/test_helpers/test_clocks?limit=10" -H "Authorization: Bearer $KEY"
```

`frozen_time` is required. A missing value is rejected with a
`parameter_missing` error naming `frozen_time`. `name` is optional and capped
at 300 characters. Lists accept `limit`, `starting_after`, and
`ending_before`, and support `expand[]` on each clock.

Creating a clock emits `test_helpers.test_clock.created`.

## Attach customers and subscriptions

A customer can join a clock when it is created, or an existing customer with no
active subscriptions can be attached when the clock is created. Attaching
stamps the customer's `created` with the clock's frozen time, like creating it
with `test_clock`. A customer belongs to at most one clock and the association
does not change afterwards.

```bash
# Create a customer on the clock. Its `created` is the frozen time.
CUSTOMER=$(curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'email=buyer@example.com' \
  --data-urlencode "test_clock=$CLOCK_ID")
CID=$(echo "$CUSTOMER" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

# Or attach an existing customer with no live subscriptions while creating
# the clock.
EXISTING=$(curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'email=existing@example.com')
EXISTING_ID=$(echo "$EXISTING" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

curl -s -X POST $BASE/v1/test_helpers/test_clocks \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'frozen_time=1577836800' \
  --data-urlencode "customer=$EXISTING_ID"
```

A `test_clock` that names no clock for the account is rejected with an
`invalid_request_error` and no customer is created. Attaching a second clock to
a customer is rejected the same way, as is attaching a customer that still has
a subscription that is not `canceled`: a clock cannot retroactively drive
subscriptions that already exist.

Customer, subscription, and invoice objects render `test_clock` and support
`expand[]=test_clock`:

```bash
curl -s "$BASE/v1/customers/$CID?expand[]=test_clock" -H "Authorization: Bearer $KEY"
```

A subscription created for a clock customer inherits the clock. Its `created`,
`current_period_start`, and item periods use the frozen time, and its first
period follows the price interval:

```bash
PRICE_ID=price_...   # a recurring price
SUB=$(curl -s -X POST $BASE/v1/subscriptions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "items[0][price]=$PRICE_ID")
SID=$(echo "$SUB" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
```

Non-clock subscriptions keep the shared 30-day first period.

## Advance a clock

```bash
# Move the clock one month ahead of its frozen time.
curl -s -X POST "$BASE/v1/test_helpers/test_clocks/$CLOCK_ID/advance" \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'frozen_time=1580428800'
```

The response reports the advance in progress:

```json
{
  "id": "clock_...",
  "object": "test_helpers.test_clock",
  "frozen_time": 1577836800,
  "status": "advancing",
  "status_details": {
    "advancing": { "target_frozen_time": 1580428800 },
    "internal_failure": null
  }
}
```

All billing work is synchronous inside one transaction; the committed clock is
already `ready` at the new time, so a retrieval that follows reports:

```bash
curl -s "$BASE/v1/test_helpers/test_clocks/$CLOCK_ID" -H "Authorization: Bearer $KEY"
# status: "ready", frozen_time: 1580428800, status_details.advancing: null
```

The target must be strictly later than the current `frozen_time`. It may be at
most two intervals of the clock's shortest subscription, or two years when the
clock has no subscriptions. The limit is measured from the current frozen time
and uses the same day approximations as billing periods (month = 30 days,
year = 365 days). Rejections are `invalid_request_error`s naming `frozen_time`.

Advancing emits `test_helpers.test_clock.advancing` at the old frozen time and
`test_helpers.test_clock.ready` at the target.

## Renewal

Advancing past a subscription's period end renews it: the period moves to the
next interval, a paid `subscription_cycle` invoice is raised at the boundary,
and the standard subscription and invoice events are emitted. Several missed
periods produce one renewal invoice each, in order.

```bash
# The subscription now covers the next month.
curl -s "$BASE/v1/subscriptions/$SID" -H "Authorization: Bearer $KEY"

# First and renewal invoices.
curl -s "$BASE/v1/invoices?subscription=$SID" -H "Authorization: Bearer $KEY"
```

Subscriptions attached to the clock are processed in period-end order, so
invoice and event order is deterministic.

## Metered usage

Metered items bill the usage reported during the period that just ended. For a
clock customer, a meter event defaults its `timestamp` to the clock's
`frozen_time` and is validated against it (35 days back, 5 minutes forward), so
usage lands inside the frozen period. Report usage as usual, then advance past
the period end; the renewal invoice carries the aggregated quantity and amount:

```bash
curl -s -X POST $BASE/v1/billing/meter_events \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'event_name=api_call' \
  --data-urlencode 'identifier=usage-1' \
  --data-urlencode "payload[stripe_customer_id]=$CID" \
  --data-urlencode 'payload[value]=7'
```

## Dunning and recovery

If a clock subscription has an open invoice from a failed collection, advancing
the clock pays that invoice before considering the next renewal. The renewal
still runs afterwards, so recovery and catch-up happen in one advance.

The clock's own advance always collects successfully. To drive a paid/failed
outcome for a single subscription, use
[`POST /internal/subscriptions/{id}/advance`](internal.md#end-a-subscription-billing-period).

## Cancellation at period end

A subscription with `cancel_at_period_end` is canceled when the clock reaches
its period end, and no renewal invoice is raised:

```bash
curl -s -X POST "$BASE/v1/subscriptions/$SID" \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'cancel_at_period_end=true'

curl -s -X POST "$BASE/v1/test_helpers/test_clocks/$CLOCK_ID/advance" \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'frozen_time=1580428800'
```

The subscription becomes `canceled` and `customer.subscription.deleted` is
emitted at the boundary. Advancing further leaves it canceled.

## Deletion

```bash
curl -s -X DELETE "$BASE/v1/test_helpers/test_clocks/$CLOCK_ID" \
  -H "Authorization: Bearer $KEY"
```

The response is a tombstone:

```json
{
  "id": "clock_...",
  "object": "test_helpers.test_clock",
  "deleted": true
}
```

Deleting a clock happens in one transaction: each live subscription is
canceled (`customer.subscription.deleted`), each customer and its saved payment
methods are deleted (`customer.deleted`), the clock is deleted, and
`test_helpers.test_clock.deleted` is emitted. Invoices remain readable and keep
their `test_clock` reference even though their customer is gone. A later
retrieval of the clock responds `not_found`.

## Timestamps and events

Clock-scoped paths stamp their objects and events with simulated time:

- `customer.created` uses the clock's `frozen_time`.
- `customer.subscription.created` and the first invoice use the frozen time.
- Checkout completion for a clock customer stamps the subscription, item
  periods, and first invoice with the frozen time; the checkout session itself
  stays wall-clock time.
- Renewal invoices and their `invoice.created`/`invoice.paid`/
  `invoice.payment_succeeded` events use the period boundary they cross.
- `customer.subscription.updated` uses the boundary (or the advance time for an
  open-invoice retry).
- `POST /internal/subscriptions/{id}/advance` on a clock subscription uses the
  clock's current `frozen_time` for the invoice and the subscription event.
- Meter events for a clock customer use the clock's `frozen_time` as their
  default `timestamp`.
- `test_helpers.test_clock.advancing` uses the old frozen time and
  `test_helpers.test_clock.ready` the target.

Unrelated paths keep wall-clock time.

## Reset

`POST /internal/reset` clears test clocks along with their associations on
customers, subscriptions, and invoices. See
[`internal.md`](internal.md#reset).
