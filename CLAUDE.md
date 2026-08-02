# ynab-cli

Open-source, **absolutely read-only** CLI for the YNAB API with first-class MCP
support. Rust. Repo: `xczimi/ynab-cli`, crate `ynab-cli`, binary `ynab`.
Modeled on `gro` (google-readonly) in spirit: small composable commands an
agent can drive via Bash, with an MCP server over the same core.

## Core principles

1. **Read-only is structural, not policy.** The client module contains only
   GET requests — no write verbs exist anywhere in the codebase, so "this
   cannot write" is verifiable by reading one file. Writes are permanently out
   of scope for this binary; if they ever happen, they live in a separate
   extension/tool.
2. **Server-side enforcement where possible.** OAuth flow always requests
   `scope=read-only`, so YNAB's server rejects writes regardless of client
   code. PAT auth is supported but unscoped (YNAB has no read-only PAT) — docs
   frame OAuth as the recommended mode, PAT as the quick start.
3. **Nothing sensitive touches disk unencrypted.** Tokens and OAuth client
   credentials live only in the OS keychain (`keyring` crate: macOS Keychain,
   Windows Credential Manager, Linux secret-service). The local cache is
   encrypted at rest. The config file never holds secrets.

## Architecture

One binary, three layers:

- **Core library**: auth, GET-only API client, domain types, cache, filters.
- **CLI frontend** (clap): thin adapters over core functions. Built first.
- **MCP frontend** (`rmcp` crate): `ynab mcp serve` (stdio), thin adapters
  over the same core functions — co-equal by design so it never drifts from
  the CLI, even though CLI commands are implemented first.

## Auth

- `ynab auth login|status|logout`. PAT paste-in implemented first (unblocks
  everything else); OAuth Authorization Code flow immediately after — both are
  v1 scope.
- **Bring-your-own OAuth app**: every user registers their own OAuth app on
  their YNAB developer page (~2 min) and hands the CLI its
  `client_id`/`client_secret` once; stored in keychain. No shared app, no
  secret in the repo or binary, ever. README carries the registration
  walkthrough. (The maintainer uses their own registration like any user.)
- OAuth endpoints: authorize `https://app.ynab.com/oauth/authorize`
  (`?client_id&redirect_uri&response_type=code&scope=read-only`), token
  `https://app.ynab.com/oauth/token`. Access tokens expire ~2h; refresh tokens
  rotate on every refresh (single-use) — always store the newest. Localhost
  redirect listener for the code; browser only for the one-time consent.

## v1 command surface

- `ynab auth login|status|logout`
- `ynab budgets list` — everything else takes `--budget <id>`, defaulting to
  config `default_budget`, then the API's `last-used` alias
- `ynab accounts list`
- `ynab categories list`
- `ynab payees list`
- `ynab transactions list` — the workhorse: `--since`/`--until`, `--payee`,
  `--account`, `--category`, `--uncategorized`, `--unapproved`. API supports
  only `since_date`; all other filters are client-side (SQL over the cache).
- `ynab config get|set` — edits the TOML config
- `ynab cache clear|status`
- `ynab mcp serve`

Deferred (on demand): months (wanted later for age-of-money), scheduled
transactions, payee locations, single-item `get` subcommands.

## Output

- Human-readable tables by default; `--json` for machine output.
- `--json` mirrors the API schema exactly — raw milliunits, no invented
  convenience fields. Human output shows real currency (milliunits / 1000;
  outflows negative).
- ISO 8601 dates everywhere, both formats.

## API layer

- Hand-rolled thin `reqwest` client against `https://api.ynab.com/v1` — only
  ~7 GET endpoints needed; deserialize only the fields we use. The generated
  `ynab-api` crate was rejected: it ships callable write methods, which would
  degrade the read-only guarantee from structural to promised.
- Rate limit: 200 requests/hour per token. On 429, print a clear "rate
  limited, resets within the hour" message — never a stack trace.

## Cache (v1, on by default)

- Delta-request cache using `last_knowledge_of_server` / `server_knowledge`,
  per budget.
- Storage: SQLite via `rusqlite` with bundled **SQLCipher** — the whole
  database is encrypted at rest; the symmetric key is generated on first run
  and stored in the OS keychain beside the tokens. Nothing on disk is readable
  without keychain access.
- Lives in the platform data dir (`~/Library/Application Support/ynab-cli/`,
  XDG, `%APPDATA%`).
- Opt-out: `cache = false` in config, or `--no-cache` per invocation — rate
  limit management is ultimately the user's choice.
- A corrupted/undecryptable cache is silently discarded and refetched; it is
  never an error the user must fix.

## Config

- TOML at the platform config dir (`~/.config/ynab-cli/config.toml`;
  `%APPDATA%` on Windows).
- v1 keys: `cache` (bool, default true), `default_budget`. Nothing else until
  needed. **No secrets in the config file, ever.**

## Distribution

- `cargo install ynab-cli` + GitHub releases. Homebrew tap later if uptake.
- License: MIT.

## Prior art (evaluated 2026-08-02, clones in ../external/)

Three existing ynab-mcp servers were rejected in favor of greenfield, but hold
patterns worth cribbing (check licenses first):

- `cinnes-ynab-mcp` (Rust): CRIB `src/secrets.rs` (keyring + secrecy + zeroize
  + Debug/Serialize redaction) and its wiremock e2e test setup. Avoid: its
  1.7k-line hand-rolled client includes writes.
- `Jtewen-ynab-mcp` (Python): CRIB the fail-closed read-only design — write
  tools both hidden from the MCP tool list AND rejected at call time. Avoid:
  wrote cleartext financial data to /tmp (our encrypted cache exists because
  of this).
- `mattweg-ynab-mcp` (Node): cautionary tale only — committed a shared OAuth
  client secret; our bring-your-own-app model exists because of this.

## Build/toolchain

- Rust via mise (mise.toml pins the toolchain); `cargo build`, `cargo test`.
- Tests: wiremock for API mocking.
- YNAB API notes: amounts are milliunits (divide by 1000), outflows negative;
  `budget_id` accepts the literal `last-used`.
