#!/usr/bin/env bash
#
# Enforces the read-only guarantee.
#
# CLAUDE.md claims the guarantee is structural rather than policy: "no HTTP
# write verb exists anywhere in our code, so 'this cannot write budget data'
# is verifiable by reading one file". Nothing enforced that. A pull request
# adding `self.http.post(...)` to the API client would have passed every
# check in this repository.
#
# The single carve-out is the OAuth token exchange, which POSTs to
# app.ynab.com/oauth/token from inside the `oauth2` crate. That is not our
# code, it cannot touch budget data, and it always requests scope=read-only.
# Our own sources must contain no write verb at all.
#
# Run locally with `just read-only`.

set -euo pipefail

failed=0

fail() {
    printf 'read-only guard FAILED: %s\n' "$1" >&2
    printf '%s\n' "$2" | sed 's/^/    /' >&2
    printf '\n' >&2
    failed=1
}

# 1. `post`, `put` and `patch` have no non-HTTP meaning in this codebase, so
#    they are banned outright.
if hits=$(grep -rnE '\.(post|put|patch)\(' src); then
    fail "HTTP write verb in src/" "$hits"
fi

# 2. `delete` is ambiguous — the keychain and the cache legitimately delete
#    things — so it is banned only in the API layer.
if hits=$(grep -rnE '\.delete\(' src/api); then
    fail "delete() in the API layer" "$hits"
fi

# 3. A method-agnostic builder would sidestep checks 1 and 2.
if hits=$(grep -rnE 'Method::(POST|PUT|PATCH|DELETE)|\.request\(' src); then
    fail "method-agnostic request builder in src/" "$hits"
fi

# 4. The strongest check: the API client must reach reqwest through exactly
#    one verb. Whitespace is stripped first because the call is written
#    across several lines. If the `http` field is ever renamed this check
#    finds no verbs at all and fails loudly rather than passing silently.
client=src/api/client.rs
verbs=$(tr -d '[:space:]' < "$client" | grep -oE '\.http\.[a-z_]+' | sort -u || true)
if [[ "$verbs" != ".http.get" ]]; then
    fail "$client must call exactly one reqwest verb, .http.get" \
         "found: ${verbs:-<none — was the http field renamed?>}"
fi

if [[ "$failed" -ne 0 ]]; then
    echo "The read-only guarantee is the entire point of this crate." >&2
    echo "If a write is genuinely needed it belongs in a separate binary." >&2
    exit 1
fi

echo "read-only guard: passed (GET is the only verb in src/)"
