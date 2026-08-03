# ynab-cli

A small, fast, **absolutely read-only** command-line client for the
[YNAB](https://www.ynab.com) API, with first-class MCP support for
AI agents. Rust, single binary (`ynab`).

## Read-only, structurally

This CLI cannot write to your budget, and that's not a policy — it's a
property of the code. The API client (`src/api/client.rs`) contains exactly
one HTTP verb, GET, and no other file in the codebase talks to the network;
"this binary cannot mutate your budget" is verifiable by reading that one
file rather than trusting a promise. On top of that, the OAuth login flow
always requests `scope=read-only`, so even if a bug somehow slipped a write
call in, YNAB's own servers would reject it. If write support is ever
wanted, it will live in a separate tool — never in `ynab-cli`.

## Install

```sh
cargo install ynab-cli
```

## Quick start (Personal Access Token)

The fastest way to get going. Generate a token from your
[YNAB Account Settings → Developer Settings](https://app.ynab.com/settings/developer)
page, then:

```sh
ynab auth login
# Paste your Personal Access Token when prompted
ynab budgets list
ynab transactions list --since 2026-01-01
```

A PAT is unscoped — YNAB doesn't offer a read-only PAT — so it can
technically authorize writes on YNAB's side even though this CLI never
issues any. For unattended use (scripts, CI) prefer OAuth (below), or set
`YNAB_PAT` in the environment instead of storing a PAT in the keychain.

## OAuth setup (recommended)

OAuth logins are scoped `read-only` by YNAB itself, so the server enforces
the read-only guarantee independently of this CLI's code. There's no shared
OAuth app — you register your own (about two minutes, once):

1. Go to your [YNAB Developer Settings](https://app.ynab.com/settings/developer)
   page and create a new OAuth Application.
2. Set the redirect URI to **exactly**:

   ```
   http://localhost:53682/callback
   ```

   (`ynab-cli` runs a one-shot local listener on that fixed port to catch the
   authorization code — the URI must match exactly or YNAB will refuse the
   redirect.)
3. Copy the generated Client ID and Client Secret.
4. Run:

   ```sh
   ynab auth login --oauth
   ```

   The CLI prompts for the client id/secret once (stored in your OS
   keychain thereafter), prints the authorization URL, opens your browser,
   and waits for the redirect. Access tokens are refreshed automatically
   when they near expiry; refresh tokens rotate on every use and only the
   newest is ever kept.

Your client id/secret are never shared, logged, or sent anywhere but
YNAB's own OAuth endpoints.

## Commands

Every command below (except `auth` and `config`) accepts the global flags
`--json`, `--budget <id>`, and `--no-cache` (see below).

| Command | Description |
|---|---|
| `ynab auth login` | Log in with a pasted Personal Access Token |
| `ynab auth login --oauth` | Log in via the OAuth browser flow |
| `ynab auth status` | Show whether you're logged in, and how |
| `ynab auth logout` | Remove all credentials and delete the local cache |
| `ynab budgets list` | List your budgets |
| `ynab accounts list` | List accounts in the current budget |
| `ynab categories list` | List category groups and categories |
| `ynab payees list` | List payees |
| `ynab transactions list` | List transactions, with filters (see below) |
| `ynab config get <key>` | Print a config value |
| `ynab config set <key> <value>` | Set a config value |
| `ynab cache status` | Show what's in the local cache |
| `ynab cache clear` | Delete the local cache |
| `ynab mcp serve` | Run the MCP server over stdio |

`transactions list` supports `--since`/`--until` (ISO dates), `--payee`,
`--account`, `--category` (id or case-insensitive name substring),
`--uncategorized`, and `--unapproved`; filters combine with AND.

Every list command that isn't budget-scoped uses `--budget <id>`, falling
back to the config's `default_budget`, falling back to the YNAB API's
`last-used` alias.

## The `--json` contract

By default, commands print human-readable tables with real currency
(milliunits divided by 1000; outflows negative) and hide deleted entities.

`--json` prints the raw API response instead: exact milliunits, no invented
convenience fields, and — for `transactions list` — the full envelope
(including `server_knowledge`) with deleted items kept, filtered only by
whatever filters you explicitly passed. `--json` output is meant to be
piped straight into `jq` or another program; it mirrors YNAB's schema, not
this CLI's opinions about it.

## Caching

By default, `ynab-cli` keeps a local, **encrypted** cache (SQLite via
bundled SQLCipher) so repeat calls use YNAB's delta sync
(`last_knowledge_of_server`) instead of refetching everything — useful
given YNAB's 200-requests/hour rate limit. The cache lives in your
platform's standard data directory and its symmetric key lives in the OS
keychain; nothing on disk is readable without keychain access.

Caching requires a **concrete budget id**. The `last-used` alias (what you
get by default on a fresh install, before you've set anything) is never
cached, because the API never reveals which budget it actually resolved to
— caching under it risks silently mixing data from different budgets. Run:

```sh
ynab config set default_budget <your-budget-id>
```

to enable it (`ynab budgets list` will show you the id).

To opt out entirely: pass `--no-cache` for one invocation, or set
`cache = false` in the config file permanently. Rate-limit management is
ultimately your call.

A corrupted or undecryptable cache file is never a user-facing error — it's
silently discarded and rebuilt on the next call.

## Environment variables

| Variable | Purpose |
|---|---|
| `YNAB_PAT` | Read a Personal Access Token from the environment instead of the keychain — handy for CI and scripts. Takes priority over a keychain-stored PAT, which takes priority over OAuth. |
| `YNAB_CLI_API_BASE_URL` | Override the YNAB API base URL (tests / self-hosted mocks). |
| `YNAB_CLI_CONFIG_DIR` | Override the config directory. |
| `YNAB_CLI_DATA_DIR` | Override the data directory (cache location). |
| `YNAB_CLI_CACHE_KEY` | Supply the cache's SQLCipher key directly instead of using the keychain — must be exactly 64 hex characters (32 bytes). |
| `YNAB_CLI_NO_BROWSER` | Skip the best-effort browser open during `auth login --oauth`; the authorization URL is always printed regardless. |

## MCP setup

`ynab mcp serve` runs an MCP server over stdio, exposing the same read-only
list operations as the CLI (`list_budgets`, `list_accounts`,
`list_categories`, `list_payees`, `list_transactions`) as MCP tools. Any
MCP client that speaks stdio can use it — for example, with the Claude Code
CLI:

```sh
claude mcp add ynab -- ynab mcp serve
```

or by pointing any other MCP-capable client at `ynab mcp serve` as a stdio
command. Authentication is whatever's already configured via `ynab auth
login` (PAT or OAuth) — the MCP server doesn't have its own separate login.

## Security model

- **Nothing sensitive touches disk unencrypted.** PAT, OAuth client
  credentials, OAuth tokens, and the cache encryption key all live only in
  the OS keychain (macOS Keychain, Windows Credential Manager, Linux
  Secret Service via `keyring`) — never in the config file, never in a
  plaintext cache.
- **The cache is encrypted at rest** with SQLCipher; its key is generated
  on first use and stored in the keychain beside your tokens.
- **`ynab auth logout` clears everything**: all stored tokens and OAuth app
  credentials, the cache database (and its journal/WAL siblings), and the
  cache encryption key. After logout, no financial data remains accessible
  on the machine.
- **Rate limiting**: YNAB allows 200 requests/hour per token. On a 429,
  `ynab-cli` prints a clear message telling you it's rate limited and will
  reset within the hour — never a raw stack trace.

## License

MIT — see [LICENSE](./LICENSE).
