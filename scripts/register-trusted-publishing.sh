#!/usr/bin/env bash
# Register a crates.io Trusted Publishing config for every publishable crate in
# this workspace, so .github/workflows/publish.yml can publish with a GitHub
# OIDC token instead of a long-lived registry token.
#
# The crates.io web UI has a form for this. It is also a plain JSON API, which
# is the only sane way to do it for a workspace this size:
#
#   POST https://crates.io/api/v1/trusted_publishing/github_configs
#   Authorization: <crates.io API token>
#   {"github_config":{"crate":"claria-core","repository_owner":"claria-ai",
#                     "repository_name":"claria","workflow_filename":"publish.yml",
#                     "environment":null}}
#
# ORDER OF OPERATIONS. crates.io resolves `crate` against an existing crate and
# checks that the caller owns it, so a config cannot be created for a name that
# has never been published. The first version of each crate has to go up with a
# real API token — once — and this script runs after that. Every later release
# publishes over OIDC with no stored secret.
#
# The token used here is only for setup and never leaves this machine. Do not
# put it in CI.
#
# Usage:
#   CARGO_REGISTRY_TOKEN=... ./scripts/register-trusted-publishing.sh
#   ./scripts/register-trusted-publishing.sh          # reads ~/.cargo/credentials.toml
set -euo pipefail

OWNER="claria-ai"
REPO="claria"
WORKFLOW="publish.yml"
API="https://crates.io/api/v1/trusted_publishing/github_configs"
UA="claria trusted-publishing setup (https://github.com/claria-ai/claria)"

TOKEN="${CARGO_REGISTRY_TOKEN:-}"
if [[ -z "$TOKEN" ]]; then
  CREDS="${CARGO_HOME:-$HOME/.cargo}/credentials.toml"
  [[ -f "$CREDS" ]] || { echo "no CARGO_REGISTRY_TOKEN and no $CREDS" >&2; exit 1; }
  TOKEN="$(sed -n 's/^ *token *= *"\(.*\)"/\1/p' "$CREDS" | head -1)"
fi
[[ -n "$TOKEN" ]] || { echo "empty crates.io token" >&2; exit 1; }

# The crate list comes from cargo, not a hardcoded array, so `publish = false`
# stays the single source of truth for what ships.
CRATES="$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print("\n".join(sorted(p["name"] for p in json.load(sys.stdin)["packages"] if p.get("publish") is None)))')"

status=0
for krate in $CRATES; do
  body="$(printf '{"github_config":{"crate":"%s","repository_owner":"%s","repository_name":"%s","workflow_filename":"%s","environment":null}}' \
    "$krate" "$OWNER" "$REPO" "$WORKFLOW")"

  response="$(curl -sS -w '\n%{http_code}' -X POST "$API" \
    -H "Authorization: $TOKEN" \
    -H 'Content-Type: application/json' \
    -A "$UA" \
    -d "$body")"
  code="${response##*$'\n'}"
  payload="${response%$'\n'*}"

  case "$code" in
    200) echo "ok       $krate -> $OWNER/$REPO@$WORKFLOW" ;;
    *)   echo "FAILED   $krate ($code): $payload" >&2; status=1 ;;
  esac
done

echo
echo "registered configs:"
for krate in $CRATES; do
  curl -sS "$API?crate=$krate" -H "Authorization: $TOKEN" -A "$UA" | python3 -c 'import json,sys
for c in json.load(sys.stdin).get("github_configs", []):
    print("  %-24s %s/%s@%s (id %s)" % (c["crate"], c["repository_owner"], c["repository_name"], c["workflow_filename"], c["id"]))'
done

exit "$status"
