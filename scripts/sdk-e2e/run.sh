#!/usr/bin/env bash
# Build the mock, start it with snapshot destinations for the platform and
# connected scopes, and run the official Stripe Node SDK end-to-end script.
# Requires `npm install` in this directory.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)

(cd "$ROOT" && cargo build --quiet)

TMP="$(mktemp -d)"
SRV=""
cleanup() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

cat > "$TMP/config.toml" <<EOF
[server]
bind = "127.0.0.1"
port = 12401
public_base_url = "http://127.0.0.1:12401"

[database]
path = "$TMP/db.sqlite"

[main_account]
email = "owner@example.com"

[[event_destinations]]
name = "platform-app"
event_payload = "snapshot"
events_from = ["@self"]
enabled_events = ["*"]
webhook_endpoint = { url = "http://127.0.0.1:12402/hook" }

[[event_destinations]]
name = "connected-app"
event_payload = "snapshot"
events_from = ["@accounts"]
enabled_events = ["*"]
webhook_endpoint = { url = "http://127.0.0.1:12403/hook" }
EOF

STRIPE_MOCK_CONFIG="$TMP/config.toml" "$ROOT/target/debug/stripe-mock" > "$TMP/server.log" 2>&1 &
SRV=$!
sleep 1

KEY=$(curl -s http://127.0.0.1:12401/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')
KEY="$KEY" node e2e.js
