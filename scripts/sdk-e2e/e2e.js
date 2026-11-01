// End-to-end verification of stripe-mock using the official Stripe Node SDK.
//
// Assumes the mock is running on 127.0.0.1:12401 with two snapshot event
// destinations:
//   platform-app  (events_from=@self)     -> 127.0.0.1:12402/hook
//   connected-app (events_from=@accounts) -> 127.0.0.1:12403/hook
// See run.sh.
const http = require('http');
const Stripe = require('stripe');

const MOCK = 'http://127.0.0.1:12401';
const key = process.env.KEY;
const stripe = new Stripe(key, { host: '127.0.0.1', port: 12401, protocol: 'http' });

function startReceiver(port) {
  const received = [];
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      received.push({ sig: req.headers['stripe-signature'], body, type: JSON.parse(body).type });
      res.writeHead(200).end('ok');
    });
  });
  server.listen(port);
  return { received, close: () => server.close() };
}

const platformReceiver = startReceiver(12402);
const connectedReceiver = startReceiver(12403);
const thinReceiver = startReceiver(12404);

const postForm = (url, params) =>
  fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams(params),
    redirect: 'manual',
  });

(async () => {
  // --- thin event destination + v2 account.created -----------------------
  const destination = await stripe.v2.core.eventDestinations.create({
    name: 'thin-e2e',
    type: 'webhook_endpoint',
    event_payload: 'thin',
    enabled_events: ['v2.core.account.created'],
    webhook_endpoint: { url: 'http://127.0.0.1:12404/hook' },
    include: ['webhook_endpoint.signing_secret'],
  });
  if (!destination.id.startsWith('ed_test_')) throw new Error('bad destination id');
  const destinationSecret = destination.webhook_endpoint.signing_secret;
  if (!destinationSecret || !destinationSecret.startsWith('whsec_')) throw new Error('missing secret');
  console.log('v2 event destination', destination.id, destination.object);

  // --- connected account + onboarding -----------------------------------
  const account = await stripe.v2.core.accounts.create({
    identity: { business_details: { name: 'SDK Acme' } },
    configuration: { merchant: { capabilities: { card_payments: { requested: true } } } },
    defaults: { currency: 'usd' },
  });
  console.log('v2 account', account.id, account.object);
  if (!account.id.startsWith('acct_')) throw new Error('bad account id');

  const link = await stripe.accountLinks.create({
    account: account.id,
    type: 'account_onboarding',
    refresh_url: 'https://example.com/refresh',
    return_url: 'https://example.com/return',
  });
  const token = link.url.split('/').pop();
  const onboard = await postForm(`${MOCK}/mock/connect/${token}`, { kyc: 'pass' });
  console.log('onboard redirect', onboard.status, onboard.headers.get('location'));
  if (onboard.status !== 303) throw new Error('onboarding did not redirect');

  // --- connected catalog + billing + checkout ----------------------------
  const opts = { stripeAccount: account.id };
  const product = await stripe.products.create({ name: 'SDK Plan' }, opts);
  const price = await stripe.prices.create(
    { product: product.id, unit_amount: 2000, currency: 'usd', recurring: { interval: 'month' } },
    opts,
  );
  const customer = await stripe.customers.create({ email: 'sdk-e2e@example.com' }, opts);
  const subscription = await stripe.subscriptions.create(
    { customer: customer.id, items: [{ price: price.id }] },
    opts,
  );
  console.log('connected subscription', subscription.id, subscription.status);

  const session = await stripe.checkout.sessions.create(
    {
      mode: 'subscription',
      customer: customer.id,
      line_items: [{ price: price.id, quantity: 1 }],
      success_url: 'https://example.com/success?session={CHECKOUT_SESSION_ID}',
      cancel_url: 'https://example.com/cancel',
    },
    opts,
  );
  const pay = await postForm(`${MOCK}/mock/checkout/${session.id}`, { card: 'success' });
  console.log('pay redirect', pay.status, pay.headers.get('location'));
  const done = await stripe.checkout.sessions.retrieve(session.id, {}, opts);
  console.log('session after', done.status, done.subscription);
  if (done.status !== 'complete') throw new Error('session not complete');

  // --- platform-scope event (main account subscription) -------------------
  const mainProduct = await stripe.products.create({ name: 'Main Plan' });
  const mainPrice = await stripe.prices.create({
    product: mainProduct.id,
    unit_amount: 500,
    currency: 'usd',
    recurring: { interval: 'month' },
  });
  const mainCustomer = await stripe.customers.create({ email: 'main@example.com' });
  await stripe.subscriptions.create({ customer: mainCustomer.id, items: [{ price: mainPrice.id }] });
  console.log('main subscription created');

  // --- customer with tax id -> invoice -> finalize -> pay -----------------
  const taxCustomer = await stripe.customers.create({
    email: 'taxed@example.com',
    tax_id_data: [{ type: 'eu_vat', value: 'DE123456789' }],
  });
  if (taxCustomer.tax_ids.length !== 1) throw new Error('customer tax id missing');
  console.log('customer with tax id', taxCustomer.id, taxCustomer.tax_ids[0].id);

  const invoice = await stripe.invoices.create({
    customer: taxCustomer.id,
    lines: [{ price: mainPrice.id, quantity: 2, tax_rate: { percentage: 10, name: 'VAT' } }],
  });
  console.log('invoice', invoice.id, invoice.status, invoice.subtotal, invoice.tax_total, invoice.total);
  if (!invoice.id.startsWith('in_')) throw new Error('bad invoice id');
  if (invoice.status !== 'draft') throw new Error('invoice not draft');
  if (invoice.tax_total !== 100) throw new Error('bad tax total');
  if (invoice.total !== 1100) throw new Error('bad invoice total');
  if (invoice.customer_tax_ids.length !== 1) throw new Error('invoice missing customer tax ids');

  const finalized = await stripe.invoices.finalizeInvoice(invoice.id);
  console.log('invoice finalized', finalized.status, finalized.amount_due);
  if (finalized.status !== 'open') throw new Error('invoice not open');

  const paid = await stripe.invoices.pay(invoice.id);
  console.log('invoice paid', paid.status, paid.amount_paid, paid.amount_remaining);
  if (paid.status !== 'paid') throw new Error('invoice not paid');
  if (paid.amount_remaining !== 0) throw new Error('invoice balance left');

  // --- settle a second invoice through the control plane --------------------
  const second = await stripe.invoices.create({
    customer: taxCustomer.id,
    lines: [{ price: mainPrice.id, quantity: 2 }],
  });
  await stripe.invoices.finalizeInvoice(second.id);
  const intent = await stripe.paymentIntents.create({
    amount: 500,
    currency: 'usd',
    customer: taxCustomer.id,
    invoice: second.id,
  });
  console.log('payment intent', intent.id, intent.status);
  if (!intent.id.startsWith('pi_')) throw new Error('bad payment intent id');

  const resolved = await (
    await fetch(`${MOCK}/internal/payment_intents/${intent.id}/resolve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ status: 'succeeded' }),
    })
  ).json();
  console.log('payment intent resolved', resolved.status);
  if (resolved.status !== 'succeeded') throw new Error('payment intent not resolved');

  const settled = await stripe.invoices.retrieve(second.id);
  console.log('invoice after resolve', settled.status, settled.amount_paid, settled.amount_remaining);
  if (settled.amount_paid !== 500) throw new Error('invoice not credited');
  if (settled.amount_remaining !== 500) throw new Error('bad remaining balance');
  if (settled.status !== 'open') throw new Error('invoice status changed on partial payment');

  // --- verify signatures through the SDK for both scopes ------------------
  const destinations = await (await fetch(`${MOCK}/internal/destinations`)).json();
  const secrets = Object.fromEntries(
    destinations.data.map((d) => [d.name, d.webhook_endpoint.signing_secret]),
  );
  await fetch(`${MOCK}/internal/deliveries/flush`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ timeout_ms: 8000 }),
  });

  const connectedEvent = connectedReceiver.received.find((r) => r.type === 'checkout.session.completed');
  const platformEvent = platformReceiver.received.find((r) => r.type === 'customer.subscription.created');
  if (!connectedEvent) throw new Error('connected-scope target received no event');
  if (!platformEvent) throw new Error('platform-scope target received no event');

  const invoiceEvent = platformReceiver.received.find((r) => r.type === 'invoice.paid');
  if (!invoiceEvent) throw new Error('no invoice.paid event received');
  const invoiceVerified = stripe.webhooks.constructEvent(invoiceEvent.body, invoiceEvent.sig, secrets['platform-app']);
  console.log('verified invoice webhook', invoiceVerified.id, invoiceVerified.type);

  const connectedVerified = stripe.webhooks.constructEvent(connectedEvent.body, connectedEvent.sig, secrets['connected-app']);
  const platformVerified = stripe.webhooks.constructEvent(platformEvent.body, platformEvent.sig, secrets['platform-app']);
  console.log('verified connected-scope webhook', connectedVerified.id, connectedVerified.account);
  console.log('verified platform-scope webhook', platformVerified.id, platformVerified.account ?? '(platform)');

  // --- verify a v2 thin event through the SDK -----------------------------
  const thinDelivery = thinReceiver.received.find((r) => r.type === 'v2.core.account.created');
  if (!thinDelivery) throw new Error('thin destination received no event');
  const thin = stripe.parseEventNotification(thinDelivery.body, thinDelivery.sig, destinationSecret);
  console.log('verified thin event', thin.id, thin.type, thin.related_object.id);
  if (thin.related_object.id !== account.id) throw new Error('thin related_object mismatch');

  const fetched = await stripe.v2.core.events.retrieve(thin.id);
  if (fetched.type !== 'v2.core.account.created' || fetched.related_object.id !== account.id) {
    throw new Error('fetched v2 event mismatch');
  }
  console.log('fetched full v2 event', fetched.id, fetched.type);

  // A ping returns a v2.core.event for the destination.
  const ping = await stripe.v2.core.eventDestinations.ping(destination.id);
  if (ping.object !== 'v2.core.event') throw new Error('bad ping event');
  console.log('destination ping event', ping.id, ping.type);

  console.log('E2E_SDK_OK');
  platformReceiver.close();
  connectedReceiver.close();
  thinReceiver.close();
})().catch((e) => {
  console.error('E2E_SDK_FAIL', e.message);
  platformReceiver.close();
  connectedReceiver.close();
  thinReceiver.close();
  process.exit(1);
});
