# M2: API Client + List Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** All five read commands work end-to-end against the YNAB API: `ynab budgets list`, `ynab accounts list`, `ynab categories list`, `ynab payees list`, and `ynab transactions list` with `--since/--until/--payee/--account/--category/--uncategorized/--unapproved` filters — human tables by default, `--json` mirroring the API schema exactly.

**Architecture:** Domain types deserialize only the fields we use, but `--json` prints the RAW response `data` object (a `serde_json::Value` kept alongside the typed parse in `ListResult<T>`), so machine output never drops fields the API returned. Commands share a `Ctx` builder (token resolution, budget resolution, base-URL test hook). All transaction filters are client-side and in-memory in M2 (SQL over the cache arrives in M3); only `since_date` is passed to the API.

**Tech Stack:** Everything from M1 plus `comfy-table = "7"` (tables). `chrono` (already a dependency) validates `--since/--until`; date comparisons are lexicographic on ISO strings.

## Global Constraints

Copied from CLAUDE.md — every task's requirements implicitly include these:

- **Read-only is structural**: `src/api/client.rs` may only call `reqwest::Client::get` — all new endpoints go through the existing `get_json` helper. No other HTTP verb anywhere.
- **`--json` mirrors the API schema exactly** — raw milliunits, no invented convenience fields, unknown fields preserved (print the raw `data` Value, never re-serialize typed structs).
- **Human output shows real currency**: milliunits / 1000, outflows negative, 2 decimals.
- **ISO 8601 dates everywhere, both formats.**
- **Budget resolution order**: `--budget` flag → config `default_budget` → the API's literal `last-used` alias.
- **Rate limit UX**: 429 keeps printing the M1 message; never a stack trace.
- **No secrets on disk or in output**; tests never touch the real keychain, network, or config dir.
- **No `.unwrap()`/`.expect()` on fallible paths in non-test code.**
- **Commits**: conventional format, no attribution trailers.
- **Gate**: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` clean at milestone end.

## Decisions locked for this plan

- **`YNAB_PAT` env var** becomes a real, documented token source (resolution: `YNAB_PAT` env → keychain → `Error::NotAuthenticated`). It enables binary-level e2e tests (the keychain mock is in-process only) and CI/scripting use. It is read-only consumption — storage stays keychain-only. Flag to the user at the milestone checkpoint as a spec addition.
- **Name-or-id filter matching**: `--payee/--account/--category <F>` keeps a transaction when F equals the id exactly OR F is a case-insensitive substring of the name.
- **Human tables filter out `deleted: true`** (and payees/accounts keep it simple: no other hiding). `--json` output is raw and unfiltered — except `transactions list`, where the user's explicit filters apply to the raw array too (unknown fields still preserved via index-paired filtering).
- **Empty results** render as the table header row only — acceptable for M2.

## Execution notes

- Branch: `m2-client-list-commands` off `main` (create in Task 1 Step 1).
- Dependency DAG: Tasks 1 and 3 start in parallel; Task 2 after 1; Task 4 after 1+3; Tasks 5 and 6 in parallel after 2+4.
- Worktree agents: FIRST `git reset --hard <sha given by controller>`, verify `src/api/client.rs` exists, then work.
- Shared-file merges (`src/lib.rs`, `src/cli/mod.rs`, `Cargo.toml`): controller resolves unions.

## File Structure

```
src/api/types.rs        — extend: Budget, Account, CategoryGroup, Category, Payee, Transaction + wrappers (Task 1)
src/api/client.rs       — extend: ListResult, get_data, get_budgets (Task 1); other endpoints (Task 2)
src/error.rs            — add Decode variant (Task 1)
src/output.rs           — milliunits, render_table, print_json (Task 3)
src/cli/context.rs      — resolve_token, resolve_budget, Ctx, build_ctx (Task 4)
src/cli/budgets.rs      — budgets list (Task 4)
src/cli/accounts.rs     — accounts list (Task 5)
src/cli/categories.rs   — categories list (Task 5)
src/cli/payees.rs       — payees list (Task 5)
src/cli/transactions.rs — transactions list + filters (Task 6)
src/cli/mod.rs          — global --json/--budget args (Task 4); command variants (Tasks 4,5,6)
tests/cli_lists.rs      — binary e2e: budgets (Task 4), transactions (Task 6)
```

---

### Task 1: Domain types + raw/typed response plumbing + `get_budgets`

**Files:**
- Modify: `src/api/types.rs`, `src/api/client.rs`, `src/error.rs`, `src/lib.rs` (no change needed — api already exported)

**Interfaces:**
- Consumes: M1's `Client::get_json`, `Error`.
- Produces (later tasks rely on these EXACT shapes):
  - `error::Error::Decode(String)` displaying as `unexpected API response: {0}`
  - In `api::types` (all structs `#[derive(Debug, Clone, serde::Deserialize)]`, all fields `pub`):
    - `Budget { id: String, name: String, first_month: Option<String>, last_month: Option<String> }`
    - `BudgetsWrapper { budgets: Vec<Budget> }`
    - `Account { id: String, name: String, #[serde(rename = "type")] kind: String, on_budget: bool, closed: bool, balance: i64, deleted: bool }`
    - `AccountsWrapper { accounts: Vec<Account> }`
    - `Category { id: String, name: String, hidden: bool, budgeted: i64, activity: i64, balance: i64, deleted: bool }`
    - `CategoryGroup { id: String, name: String, hidden: bool, deleted: bool, categories: Vec<Category> }`
    - `CategoryGroupsWrapper { category_groups: Vec<CategoryGroup> }`
    - `Payee { id: String, name: String, deleted: bool }`
    - `PayeesWrapper { payees: Vec<Payee> }`
    - `Transaction { id: String, date: String, amount: i64, memo: Option<String>, approved: bool, account_id: String, account_name: Option<String>, payee_id: Option<String>, payee_name: Option<String>, category_id: Option<String>, category_name: Option<String>, deleted: bool }`
    - `TransactionsWrapper { transactions: Vec<Transaction> }`
  - In `api::client`:
    - `pub struct ListResult<T> { pub raw: serde_json::Value, pub parsed: T }` — `raw` is the response's `data` object verbatim.
    - `pub(crate) async fn get_data<T: DeserializeOwned>(&self, path: &str) -> Result<ListResult<T>>`
    - `pub async fn get_budgets(&self) -> Result<ListResult<BudgetsWrapper>>` — GET `/budgets`

- [ ] **Step 1: Create the branch**

```bash
git checkout -b m2-client-list-commands
```

- [ ] **Step 2: Write the failing tests**

Append to the test module in `src/api/client.rs` (it already has `client(&server)` helper from M1):

```rust
    #[tokio::test]
    async fn get_budgets_parses_and_keeps_raw() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets"))
            .and(header("Authorization", "Bearer tok-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "budgets": [
                        { "id": "b-1", "name": "Family", "first_month": "2025-01-01",
                          "last_month": "2026-08-01",
                          "some_future_field": { "kept": true } }
                    ],
                    "default_budget": null
                }
            })))
            .mount(&server)
            .await;

        let result = client(&server).get_budgets().await.unwrap();
        assert_eq!(result.parsed.budgets.len(), 1);
        assert_eq!(result.parsed.budgets[0].name, "Family");
        // raw keeps fields the typed parse ignores
        assert_eq!(result.raw["budgets"][0]["some_future_field"]["kept"], true);
        assert!(result.raw.get("default_budget").is_some());
    }

    #[tokio::test]
    async fn get_data_missing_envelope_is_decode_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"nope": 1})))
            .mount(&server)
            .await;

        let err = client(&server).get_budgets().await.unwrap_err();
        assert!(matches!(err, crate::error::Error::Decode(_)));
    }
```

Add a types round-trip test module at the bottom of `src/api/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_parses_from_api_shape() {
        let value = serde_json::json!({
            "id": "t-1", "date": "2026-07-01", "amount": -12340,
            "memo": null, "cleared": "cleared", "approved": false,
            "account_id": "a-1", "account_name": "Chequing",
            "payee_id": "p-1", "payee_name": "Grocer",
            "category_id": null, "category_name": null,
            "deleted": false, "subtransactions": []
        });
        let t: Transaction = serde_json::from_value(value).unwrap();
        assert_eq!(t.amount, -12340);
        assert!(t.category_id.is_none());
        assert_eq!(t.payee_name.as_deref(), Some("Grocer"));
    }

    #[test]
    fn category_groups_nest() {
        let value = serde_json::json!({
            "category_groups": [
                { "id": "g-1", "name": "Bills", "hidden": false, "deleted": false,
                  "categories": [
                    { "id": "c-1", "name": "Rent", "hidden": false,
                      "budgeted": 1500000, "activity": -1500000, "balance": 0,
                      "deleted": false }
                  ] }
            ]
        });
        let w: CategoryGroupsWrapper = serde_json::from_value(value).unwrap();
        assert_eq!(w.category_groups[0].categories[0].budgeted, 1_500_000);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test api`
Expected: FAIL to compile — types and `get_budgets` not defined.

- [ ] **Step 4: Write minimal implementation**

Add to `src/error.rs` enum (after the `Api` variant):

```rust
    #[error("unexpected API response: {0}")]
    Decode(String),
```

Append to `src/api/types.rs`:

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Budget {
    pub id: String,
    pub name: String,
    pub first_month: Option<String>,
    pub last_month: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BudgetsWrapper {
    pub budgets: Vec<Budget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub on_budget: bool,
    pub closed: bool,
    pub balance: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountsWrapper {
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub hidden: bool,
    pub budgeted: i64,
    pub activity: i64,
    pub balance: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryGroup {
    pub id: String,
    pub name: String,
    pub hidden: bool,
    pub deleted: bool,
    pub categories: Vec<Category>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryGroupsWrapper {
    pub category_groups: Vec<CategoryGroup>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Payee {
    pub id: String,
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayeesWrapper {
    pub payees: Vec<Payee>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub date: String,
    pub amount: i64,
    pub memo: Option<String>,
    pub approved: bool,
    pub account_id: String,
    pub account_name: Option<String>,
    pub payee_id: Option<String>,
    pub payee_name: Option<String>,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransactionsWrapper {
    pub transactions: Vec<Transaction>,
}
```

In `src/api/client.rs` add (after `get_user`):

```rust
    /// GET `path`, keep the raw `data` object AND a typed parse of it.
    /// Raw is what `--json` prints — the API schema, mirrored exactly.
    pub(crate) async fn get_data<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<ListResult<T>> {
        let value: serde_json::Value = self.get_json(path).await?;
        let raw = value
            .get("data")
            .cloned()
            .ok_or_else(|| Error::Decode("missing data envelope".into()))?;
        let parsed = serde_json::from_value(raw.clone())
            .map_err(|e| Error::Decode(e.to_string()))?;
        Ok(ListResult { raw, parsed })
    }

    pub async fn get_budgets(&self) -> Result<ListResult<BudgetsWrapper>> {
        self.get_data("/budgets").await
    }
```

with, above the `impl` block:

```rust
pub struct ListResult<T> {
    pub raw: serde_json::Value,
    pub parsed: T,
}
```

and extend the file's imports: `use crate::api::types::{BudgetsWrapper, DataEnvelope, ErrorEnvelope, User, UserWrapper};` (adjust to what the file actually uses — `DataEnvelope` stays for `get_user`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test api`
Expected: PASS (M1's 4 + 4 new).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: domain types, raw+typed ListResult plumbing, budgets endpoint"
```

---

### Task 2: Remaining endpoints (accounts, categories, payees, transactions)

**Files:**
- Modify: `src/api/client.rs`

**Interfaces:**
- Consumes: Task 1's `get_data`, wrappers.
- Produces (exact signatures Tasks 5/6 call):
  - `pub async fn get_accounts(&self, budget: &str) -> Result<ListResult<AccountsWrapper>>` — GET `/budgets/{budget}/accounts`
  - `pub async fn get_categories(&self, budget: &str) -> Result<ListResult<CategoryGroupsWrapper>>` — GET `/budgets/{budget}/categories`
  - `pub async fn get_payees(&self, budget: &str) -> Result<ListResult<PayeesWrapper>>` — GET `/budgets/{budget}/payees`
  - `pub async fn get_transactions(&self, budget: &str, since_date: Option<&str>) -> Result<ListResult<TransactionsWrapper>>` — GET `/budgets/{budget}/transactions` (+ `?since_date=YYYY-MM-DD` when Some)

- [ ] **Step 1: Write the failing tests**

Append to the client test module:

```rust
    #[tokio::test]
    async fn get_accounts_hits_budget_scoped_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing", "type": "checking",
                      "on_budget": true, "closed": false, "balance": 100500,
                      "deleted": false }
                ], "server_knowledge": 42 }
            })))
            .mount(&server)
            .await;

        let r = client(&server).get_accounts("b-1").await.unwrap();
        assert_eq!(r.parsed.accounts[0].kind, "checking");
        assert_eq!(r.raw["server_knowledge"], 42);
    }

    #[tokio::test]
    async fn get_transactions_passes_since_date() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/last-used/transactions"))
            .and(wiremock::matchers::query_param("since_date", "2026-07-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "transactions": [], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        let r = client(&server)
            .get_transactions("last-used", Some("2026-07-01"))
            .await
            .unwrap();
        assert!(r.parsed.transactions.is_empty());
    }

    #[tokio::test]
    async fn get_categories_and_payees_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "category_groups": [], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/payees"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "payees": [
                    { "id": "p-1", "name": "Grocer", "deleted": false }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        let c = client(&server).get_categories("b-1").await.unwrap();
        assert!(c.parsed.category_groups.is_empty());
        let p = client(&server).get_payees("b-1").await.unwrap();
        assert_eq!(p.parsed.payees[0].name, "Grocer");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test api`
Expected: FAIL to compile — methods not defined.

- [ ] **Step 3: Write minimal implementation**

Append inside `impl Client` (extend the types import accordingly):

```rust
    pub async fn get_accounts(&self, budget: &str) -> Result<ListResult<AccountsWrapper>> {
        self.get_data(&format!("/budgets/{budget}/accounts")).await
    }

    pub async fn get_categories(
        &self,
        budget: &str,
    ) -> Result<ListResult<CategoryGroupsWrapper>> {
        self.get_data(&format!("/budgets/{budget}/categories")).await
    }

    pub async fn get_payees(&self, budget: &str) -> Result<ListResult<PayeesWrapper>> {
        self.get_data(&format!("/budgets/{budget}/payees")).await
    }

    pub async fn get_transactions(
        &self,
        budget: &str,
        since_date: Option<&str>,
    ) -> Result<ListResult<TransactionsWrapper>> {
        let path = match since_date {
            Some(d) => format!("/budgets/{budget}/transactions?since_date={d}"),
            None => format!("/budgets/{budget}/transactions"),
        };
        self.get_data(&path).await
    }
```

(Still only `get_json` → `reqwest::Client::get` under the hood — the read-only guarantee is untouched.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test api`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: accounts/categories/payees/transactions GET endpoints"
```

---

### Task 3: Output module (currency, tables, JSON)

**Files:**
- Create: `src/output.rs`
- Modify: `src/lib.rs` (add `pub mod output;`), `Cargo.toml` (add `comfy-table = "7"`)

**Interfaces:**
- Consumes: nothing from other M2 tasks (parallel-safe).
- Produces:
  - `pub fn milliunits(amount: i64) -> String` — `-12340` → `"-12.34"`, `0` → `"0.00"`, `1005` → `"1.01"` (round half away from zero), `-999999` → `"-1000.00"`.
  - `pub fn render_table(headers: &[&str], rows: Vec<Vec<String>>) -> String` — comfy-table with `presets::UTF8_BORDERS_ONLY`, header row bolded off (plain), content left as given.
  - `pub fn print_json(value: &serde_json::Value) -> crate::error::Result<()>` — pretty-print to stdout.

- [ ] **Step 1: Write the failing test**

Create `src/output.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliunits_formatting() {
        assert_eq!(milliunits(0), "0.00");
        assert_eq!(milliunits(-12340), "-12.34");
        assert_eq!(milliunits(100500), "100.50");
        assert_eq!(milliunits(1005), "1.01");
        assert_eq!(milliunits(-999999), "-1000.00");
        assert_eq!(milliunits(-5), "-0.01");
        assert_eq!(milliunits(4), "0.00");
    }

    #[test]
    fn table_contains_headers_and_cells() {
        let out = render_table(
            &["Name", "Balance"],
            vec![vec!["Chequing".into(), "100.50".into()]],
        );
        assert!(out.contains("Name"));
        assert!(out.contains("Chequing"));
        assert!(out.contains("100.50"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test output`
Expected: FAIL to compile.

- [ ] **Step 3: Write minimal implementation**

Add `comfy-table = "7"` to `[dependencies]`. Prepend to `src/output.rs`:

```rust
use comfy_table::{presets, Table};

/// Milliunits → currency string: divide by 1000, 2 decimals,
/// round half away from zero, outflows keep their minus sign.
pub fn milliunits(amount: i64) -> String {
    let sign = if amount < 0 { "-" } else { "" };
    let abs = amount.unsigned_abs();
    let rounded = (abs + 5) / 10; // hundredths
    let whole = rounded / 100;
    let cents = rounded % 100;
    if whole == 0 && cents == 0 {
        return "0.00".to_string();
    }
    format!("{sign}{whole}.{cents:02}")
}

pub fn render_table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_BORDERS_ONLY);
    table.set_header(headers.to_vec());
    for row in rows {
        table.add_row(row);
    }
    table.to_string()
}

pub fn print_json(value: &serde_json::Value) -> crate::error::Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| crate::error::Error::Decode(e.to_string()))?;
    println!("{text}");
    Ok(())
}
```

NOTE: this uses `Error::Decode` from Task 1. If Task 1 has not merged yet in your workspace, add the variant yourself with the identical line — the controller merges unions. Add `pub mod output;` to `src/lib.rs`.

Edge check on `milliunits(-4)`: `abs=4, rounded=0, whole=0, cents=0` → returns `"0.00"` (no `-0.00`), which the first test's `milliunits(4)` covers symmetrically.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test output`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: output module with milliunit currency, tables, raw JSON"
```

---

### Task 4: Command context + global flags + `budgets list`

**Files:**
- Create: `src/cli/context.rs`, `src/cli/budgets.rs`, `tests/cli_lists.rs`
- Modify: `src/cli/mod.rs`

**Interfaces:**
- Consumes: Task 1 (`Client::get_budgets`, `ListResult`), Task 3 (`output::*`), M1 (`SecretStore`, `Config`).
- Produces:
  - `cli::context::resolve_token(store: &SecretStore) -> Result<secrecy::SecretString>` — env `YNAB_PAT` (non-empty) → keychain PAT → `Err(Error::NotAuthenticated)`.
  - `cli::context::resolve_budget(flag: Option<&str>, config: &Config) -> String` — flag → `config.default_budget` → `"last-used"`.
  - `cli::context::Ctx { pub client: api::client::Client, pub json: bool, pub budget: String }`
  - `cli::context::build_ctx(json: bool, budget_flag: Option<&str>) -> Result<Ctx>` — reads config, resolves token+budget, honors `YNAB_CLI_API_BASE_URL` env hook (the CLI frontend owns env hooks — the M4 MCP frontend will make its own choice; comment this).
  - `cli::budgets::list(ctx: &Ctx) -> Result<()>` (async)
  - `cli::Cli` gains global args: `#[arg(long, global = true)] pub json: bool` and `#[arg(long, global = true, value_name = "BUDGET")] pub budget: Option<String>`.
  - `cli::Command` gains `Budgets { command: BudgetsCommand }` with `#[derive(Debug, Subcommand)] pub enum BudgetsCommand { List }`; `run` passes `cli.json`/`cli.budget` down.
- **Behavior:** human mode prints a table `Name | Id | First Month | Last Month` (months show `-` when absent); `--json` prints the raw `data` object.

- [ ] **Step 1: Write the failing unit tests**

Create `src/cli/context.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::secrets::{SecretKind, SecretStore};
    use secrecy::{ExposeSecret, SecretString};

    fn mock_store() -> SecretStore {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        SecretStore::new().unwrap()
    }

    #[test]
    fn budget_resolution_order() {
        let cfg = Config { cache: None, default_budget: Some("cfg-b".into()) };
        assert_eq!(resolve_budget(Some("flag-b"), &cfg), "flag-b");
        assert_eq!(resolve_budget(None, &cfg), "cfg-b");
        let empty = Config::default();
        assert_eq!(resolve_budget(None, &empty), "last-used");
    }

    #[test]
    fn token_prefers_keychain_when_no_env() {
        // Serial-safety: this test must not run with YNAB_PAT set; the
        // binary-level test in tests/cli_lists.rs covers the env path.
        let store = mock_store();
        store.set(SecretKind::Pat, SecretString::from("kc-tok")).unwrap();
        let tok = resolve_token(&store).unwrap();
        assert_eq!(tok.expose_secret(), "kc-tok");
    }

    #[test]
    fn token_missing_is_not_authenticated() {
        let store = mock_store();
        let err = resolve_token(&store).unwrap_err();
        assert!(matches!(err, crate::error::Error::NotAuthenticated));
    }
}
```

- [ ] **Step 2: Write the failing binary e2e test**

Create `tests/cli_lists.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ynab(config_dir: &std::path::Path, base_url: &str) -> Command {
    let mut cmd = Command::cargo_bin("ynab").unwrap();
    cmd.env("YNAB_CLI_CONFIG_DIR", config_dir);
    cmd.env("YNAB_CLI_API_BASE_URL", base_url);
    cmd.env("YNAB_PAT", "e2e-token");
    cmd
}

#[tokio::test(flavor = "multi_thread")]
async fn budgets_list_renders_table_and_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets"))
        .and(header("Authorization", "Bearer e2e-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "budgets": [
                { "id": "b-1", "name": "Family", "first_month": "2025-01-01",
                  "last_month": "2026-08-01", "extra_api_field": 7 }
            ], "default_budget": null }
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        let dir = tempfile::tempdir().unwrap();
        // human table
        ynab(dir.path(), &uri)
            .args(["budgets", "list"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Family"))
            .stdout(predicate::str::contains("b-1"));
        // raw json mirrors schema including unknown fields
        ynab(dir.path(), &uri)
            .args(["budgets", "list", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"extra_api_field\": 7"));
    })
    .await
    .unwrap();
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test context; cargo test --test cli_lists`
Expected: FAIL to compile / unknown subcommand.

- [ ] **Step 4: Write minimal implementation**

Prepend to `src/cli/context.rs`:

```rust
use secrecy::SecretString;

use crate::api::client::Client;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::secrets::{SecretKind, SecretStore};

pub struct Ctx {
    pub client: Client,
    pub json: bool,
    pub budget: String,
}

/// Token sources in order: YNAB_PAT env var (CI/scripts), then the OS
/// keychain. Storage remains keychain-only — the env var is read, never
/// written.
pub fn resolve_token(store: &SecretStore) -> Result<SecretString> {
    if let Ok(tok) = std::env::var("YNAB_PAT") {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            return Ok(SecretString::from(trimmed.to_string()));
        }
    }
    store.get(SecretKind::Pat)?.ok_or(Error::NotAuthenticated)
}

pub fn resolve_budget(flag: Option<&str>, config: &Config) -> String {
    if let Some(b) = flag {
        return b.to_string();
    }
    config
        .default_budget
        .clone()
        .unwrap_or_else(|| "last-used".to_string())
}

/// Env hooks (YNAB_CLI_API_BASE_URL, YNAB_PAT) are a CLI-frontend concern;
/// the M4 MCP frontend decides its own wiring.
pub fn build_ctx(json: bool, budget_flag: Option<&str>) -> Result<Ctx> {
    let config = Config::load()?;
    let store = SecretStore::new()?;
    let token = resolve_token(&store)?;
    let client = match std::env::var("YNAB_CLI_API_BASE_URL").ok() {
        Some(base) => Client::with_base_url(token, base),
        None => Client::new(token),
    };
    Ok(Ctx { client, json, budget: resolve_budget(budget_flag, &config) })
}
```

Create `src/cli/budgets.rs`:

```rust
use crate::cli::context::Ctx;
use crate::error::Result;
use crate::output;

pub async fn list(ctx: &Ctx) -> Result<()> {
    let result = ctx.client.get_budgets().await?;
    if ctx.json {
        return output::print_json(&result.raw);
    }
    let rows = result
        .parsed
        .budgets
        .iter()
        .map(|b| {
            vec![
                b.name.clone(),
                b.id.clone(),
                b.first_month.clone().unwrap_or_else(|| "-".to_string()),
                b.last_month.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    println!(
        "{}",
        output::render_table(&["Name", "Id", "First Month", "Last Month"], rows)
    );
    Ok(())
}
```

In `src/cli/mod.rs`: add `pub mod budgets;` and `pub mod context;`; add to `Cli`:

```rust
    /// Output raw API JSON instead of a table
    #[arg(long, global = true)]
    pub json: bool,
    /// Budget id (default: config default_budget, then the API's last-used)
    #[arg(long, global = true, value_name = "BUDGET")]
    pub budget: Option<String>,
```

add to `Command`:

```rust
    /// List budgets
    Budgets {
        #[command(subcommand)]
        command: BudgetsCommand,
    },
```

with:

```rust
#[derive(Debug, Subcommand)]
pub enum BudgetsCommand {
    /// List all budgets
    List,
}
```

and in `run` (destructure the globals first: `let json = cli.json; let budget = cli.budget.clone();`):

```rust
        Command::Budgets { command } => match command {
            BudgetsCommand::List => {
                let ctx = context::build_ctx(json, budget.as_deref())?;
                budgets::list(&ctx).await
            }
        },
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS (everything, including the new e2e — no keychain prompt because YNAB_PAT bypasses the store in the spawned binary).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: command context, global --json/--budget flags, budgets list"
```

---

### Task 5: `accounts list`, `categories list`, `payees list`

**Files:**
- Create: `src/cli/accounts.rs`, `src/cli/categories.rs`, `src/cli/payees.rs`
- Modify: `src/cli/mod.rs`

**Interfaces:**
- Consumes: Task 2 endpoints, Task 3 output, Task 4 `Ctx`/`build_ctx` and the dispatch pattern.
- Produces: `accounts::list(ctx) `, `categories::list(ctx)`, `payees::list(ctx)` — all `pub async fn list(ctx: &Ctx) -> Result<()>`; `Command` variants `Accounts/Categories/Payees` each with a `List` subcommand enum (`AccountsCommand`, `CategoriesCommand`, `PayeesCommand` — same shape as `BudgetsCommand`).

**Behavior (exact):**
- All three resolve the budget via `ctx.budget` and call their endpoint.
- `--json`: print `result.raw` unfiltered and return.
- Human tables (skip `deleted: true` rows):
  - accounts: `Name | Type | Balance | Closed` — Balance via `output::milliunits`, Closed shows `yes`/`no`.
  - categories: `Group | Category | Budgeted | Activity | Balance` — one row per non-deleted category in each non-deleted group, milliunits for the three amounts; hidden categories/groups still shown.
  - payees: `Name | Id` — skip deleted.

- [ ] **Step 1: Write the failing unit tests**

Each new file gets a test module driving `list` through a wiremock-backed `Ctx`. Shown for accounts; categories and payees follow the identical pattern with their own sample payloads (categories: one group "Bills" containing "Rent" budgeted 1_500_000 plus one `"deleted": true` group that must not appear; payees: "Grocer" plus a deleted payee that must not appear):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::Client;
    use crate::cli::context::Ctx;
    use secrecy::SecretString;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ctx(server: &MockServer, json: bool) -> Ctx {
        Ctx {
            client: Client::with_base_url(SecretString::from("t"), server.uri()),
            json,
            budget: "b-1".to_string(),
        }
    }

    #[tokio::test]
    async fn accounts_list_human_skips_deleted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/accounts"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "accounts": [
                    { "id": "a-1", "name": "Chequing", "type": "checking",
                      "on_budget": true, "closed": false, "balance": 100500,
                      "deleted": false },
                    { "id": "a-2", "name": "Old", "type": "savings",
                      "on_budget": true, "closed": true, "balance": 0,
                      "deleted": true }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        // The function prints; success + correct route + no panic is the
        // contract at unit level (stdout assertions live in the e2e tests).
        list(&ctx(&server, false)).await.unwrap();
        list(&ctx(&server, true)).await.unwrap();
    }
}
```

(Stdout-content assertions live at the binary level in Task 4/6's e2e tests; unit level asserts behavior/route correctness. Do NOT capture stdout with extra dev-deps — keep it simple.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test accounts categories payees` (or `cargo test cli::`)
Expected: FAIL to compile.

- [ ] **Step 3: Write minimal implementation**

`src/cli/accounts.rs`:

```rust
use crate::cli::context::Ctx;
use crate::error::Result;
use crate::output;

pub async fn list(ctx: &Ctx) -> Result<()> {
    let result = ctx.client.get_accounts(&ctx.budget).await?;
    if ctx.json {
        return output::print_json(&result.raw);
    }
    let rows = result
        .parsed
        .accounts
        .iter()
        .filter(|a| !a.deleted)
        .map(|a| {
            vec![
                a.name.clone(),
                a.kind.clone(),
                output::milliunits(a.balance),
                if a.closed { "yes" } else { "no" }.to_string(),
            ]
        })
        .collect();
    println!(
        "{}",
        output::render_table(&["Name", "Type", "Balance", "Closed"], rows)
    );
    Ok(())
}
```

`src/cli/categories.rs`:

```rust
use crate::cli::context::Ctx;
use crate::error::Result;
use crate::output;

pub async fn list(ctx: &Ctx) -> Result<()> {
    let result = ctx.client.get_categories(&ctx.budget).await?;
    if ctx.json {
        return output::print_json(&result.raw);
    }
    let mut rows = Vec::new();
    for group in result.parsed.category_groups.iter().filter(|g| !g.deleted) {
        for cat in group.categories.iter().filter(|c| !c.deleted) {
            rows.push(vec![
                group.name.clone(),
                cat.name.clone(),
                output::milliunits(cat.budgeted),
                output::milliunits(cat.activity),
                output::milliunits(cat.balance),
            ]);
        }
    }
    println!(
        "{}",
        output::render_table(
            &["Group", "Category", "Budgeted", "Activity", "Balance"],
            rows
        )
    );
    Ok(())
}
```

`src/cli/payees.rs`:

```rust
use crate::cli::context::Ctx;
use crate::error::Result;
use crate::output;

pub async fn list(ctx: &Ctx) -> Result<()> {
    let result = ctx.client.get_payees(&ctx.budget).await?;
    if ctx.json {
        return output::print_json(&result.raw);
    }
    let rows = result
        .parsed
        .payees
        .iter()
        .filter(|p| !p.deleted)
        .map(|p| vec![p.name.clone(), p.id.clone()])
        .collect();
    println!("{}", output::render_table(&["Name", "Id"], rows));
    Ok(())
}
```

`src/cli/mod.rs`: add the three `pub mod`s, three `Command` variants + subcommand enums (exact `BudgetsCommand` pattern), and three dispatch arms calling `context::build_ctx(json, budget.as_deref())?` then `<module>::list(&ctx).await`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: accounts/categories/payees list commands"
```

---

### Task 6: `transactions list` with filters

**Files:**
- Create: `src/cli/transactions.rs`
- Modify: `src/cli/mod.rs`, `tests/cli_lists.rs` (append e2e)

**Interfaces:**
- Consumes: Task 2 `get_transactions`, Task 3 output, Task 4 `Ctx`.
- Produces:
  - `pub struct Filters { pub since: Option<String>, pub until: Option<String>, pub payee: Option<String>, pub account: Option<String>, pub category: Option<String>, pub uncategorized: bool, pub unapproved: bool }`
  - `pub async fn list(ctx: &Ctx, filters: Filters) -> Result<()>`
  - `pub(crate) fn keep(t: &crate::api::types::Transaction, f: &Filters) -> bool` — pure, unit-tested.
  - `Command::Transactions { command: TransactionsCommand }`; `TransactionsCommand::List { ... }` carries the seven clap args (`#[arg(long)]` each; bools are flags).

**Behavior (exact):**
- Validate `--since`/`--until` with `chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d")`; on failure: `Error::Config("--since must be an ISO date (YYYY-MM-DD)")` (same message pattern for `--until`).
- `since` is passed to the API as `since_date`; every other filter is client-side via `keep`.
- `keep` logic: skip `deleted`; `until`: `t.date.as_str() <= until` (ISO strings compare correctly); name-or-id match for payee/account/category: filter string equals the id (`Some(f) == t.payee_id.as_deref()`) OR is a case-insensitive substring of the name; `uncategorized`: `t.category_id.is_none()`; `unapproved`: `!t.approved`.
- Human table: `Date | Account | Payee | Category | Memo | Amount` (empty optionals → `-`; memo truncated to 40 chars with `…`), amount via `output::milliunits`.
- `--json`: filter the RAW array in index-lockstep with the typed one (`raw["transactions"]` and `parsed.transactions` come from the same array), then print `{"transactions": [kept raw items]}` — unknown fields preserved.

- [ ] **Step 1: Write the failing unit tests**

In `src/cli/transactions.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::Transaction;

    fn tx() -> Transaction {
        Transaction {
            id: "t-1".into(),
            date: "2026-07-15".into(),
            amount: -12340,
            memo: Some("weekly shop".into()),
            approved: true,
            account_id: "a-1".into(),
            account_name: Some("Chequing".into()),
            payee_id: Some("p-1".into()),
            payee_name: Some("Corner Grocer".into()),
            category_id: Some("c-1".into()),
            category_name: Some("Groceries".into()),
            deleted: false,
        }
    }

    #[test]
    fn keep_defaults_true_and_skips_deleted() {
        let f = Filters::default();
        assert!(keep(&tx(), &f));
        let mut dead = tx();
        dead.deleted = true;
        assert!(!keep(&dead, &f));
    }

    #[test]
    fn until_is_inclusive_lexicographic() {
        let f = Filters { until: Some("2026-07-15".into()), ..Default::default() };
        assert!(keep(&tx(), &f));
        let f = Filters { until: Some("2026-07-14".into()), ..Default::default() };
        assert!(!keep(&tx(), &f));
    }

    #[test]
    fn payee_matches_id_or_name_substring() {
        let f = Filters { payee: Some("p-1".into()), ..Default::default() };
        assert!(keep(&tx(), &f));
        let f = Filters { payee: Some("grocer".into()), ..Default::default() };
        assert!(keep(&tx(), &f));
        let f = Filters { payee: Some("landlord".into()), ..Default::default() };
        assert!(!keep(&tx(), &f));
    }

    #[test]
    fn uncategorized_and_unapproved() {
        let f = Filters { uncategorized: true, ..Default::default() };
        assert!(!keep(&tx(), &f));
        let mut t = tx();
        t.category_id = None;
        assert!(keep(&t, &f));

        let f = Filters { unapproved: true, ..Default::default() };
        assert!(!keep(&tx(), &f));
        let mut t = tx();
        t.approved = false;
        assert!(keep(&t, &f));
    }

    #[test]
    fn date_validation() {
        assert!(validate_date("2026-07-01", "--since").is_ok());
        let err = validate_date("07/01/2026", "--since").unwrap_err();
        assert_eq!(
            err.to_string(),
            "config error: --since must be an ISO date (YYYY-MM-DD)"
        );
    }
}
```

- [ ] **Step 2: Append the failing e2e test** to `tests/cli_lists.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn transactions_list_filters_and_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/budgets/last-used/transactions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "transactions": [
                { "id": "t-1", "date": "2026-07-15", "amount": -12340,
                  "memo": "weekly shop", "approved": true,
                  "account_id": "a-1", "account_name": "Chequing",
                  "payee_id": "p-1", "payee_name": "Corner Grocer",
                  "category_id": "c-1", "category_name": "Groceries",
                  "deleted": false, "subtransactions": [] },
                { "id": "t-2", "date": "2026-07-20", "amount": -5000,
                  "memo": null, "approved": false,
                  "account_id": "a-1", "account_name": "Chequing",
                  "payee_id": null, "payee_name": "Mystery",
                  "category_id": null, "category_name": null,
                  "deleted": false, "subtransactions": [] }
            ], "server_knowledge": 9 }
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        let dir = tempfile::tempdir().unwrap();
        // human: uncategorized filter keeps only t-2, shows currency
        ynab(dir.path(), &uri)
            .args(["transactions", "list", "--uncategorized"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Mystery"))
            .stdout(predicate::str::contains("-5.00"))
            .stdout(predicate::str::contains("Corner Grocer").not());
        // json: filtered raw array preserves unknown fields (subtransactions)
        ynab(dir.path(), &uri)
            .args(["transactions", "list", "--uncategorized", "--json"])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"t-2\""))
            .stdout(predicate::str::contains("subtransactions"))
            .stdout(predicate::str::contains("\"t-1\"").not());
        // bad date errors cleanly
        ynab(dir.path(), &uri)
            .args(["transactions", "list", "--since", "yesterday"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("ISO date"));
    })
    .await
    .unwrap();
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test transactions; cargo test --test cli_lists`
Expected: FAIL to compile / unknown subcommand.

- [ ] **Step 4: Write minimal implementation**

`src/cli/transactions.rs` (above the tests):

```rust
use crate::api::types::Transaction;
use crate::cli::context::Ctx;
use crate::error::{Error, Result};
use crate::output;

#[derive(Debug, Default)]
pub struct Filters {
    pub since: Option<String>,
    pub until: Option<String>,
    pub payee: Option<String>,
    pub account: Option<String>,
    pub category: Option<String>,
    pub uncategorized: bool,
    pub unapproved: bool,
}

pub(crate) fn validate_date(value: &str, flag: &str) -> Result<()> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| Error::Config(format!("{flag} must be an ISO date (YYYY-MM-DD)")))
}

fn name_or_id(filter: &str, id: Option<&str>, name: Option<&str>) -> bool {
    if Some(filter) == id {
        return true;
    }
    name.map(|n| n.to_lowercase().contains(&filter.to_lowercase()))
        .unwrap_or(false)
}

pub(crate) fn keep(t: &Transaction, f: &Filters) -> bool {
    if t.deleted {
        return false;
    }
    if let Some(until) = &f.until {
        if t.date.as_str() > until.as_str() {
            return false;
        }
    }
    if let Some(p) = &f.payee {
        if !name_or_id(p, t.payee_id.as_deref(), t.payee_name.as_deref()) {
            return false;
        }
    }
    if let Some(a) = &f.account {
        if !name_or_id(a, Some(t.account_id.as_str()), t.account_name.as_deref()) {
            return false;
        }
    }
    if let Some(c) = &f.category {
        if !name_or_id(c, t.category_id.as_deref(), t.category_name.as_deref()) {
            return false;
        }
    }
    if f.uncategorized && t.category_id.is_some() {
        return false;
    }
    if f.unapproved && t.approved {
        return false;
    }
    true
}

fn truncate_memo(memo: &Option<String>) -> String {
    match memo {
        None => "-".to_string(),
        Some(m) if m.chars().count() <= 40 => m.clone(),
        Some(m) => {
            let cut: String = m.chars().take(40).collect();
            format!("{cut}…")
        }
    }
}

pub async fn list(ctx: &Ctx, filters: Filters) -> Result<()> {
    if let Some(s) = &filters.since {
        validate_date(s, "--since")?;
    }
    if let Some(u) = &filters.until {
        validate_date(u, "--until")?;
    }
    let result = ctx
        .client
        .get_transactions(&ctx.budget, filters.since.as_deref())
        .await?;

    let kept: Vec<bool> = result
        .parsed
        .transactions
        .iter()
        .map(|t| keep(t, &filters))
        .collect();

    if ctx.json {
        let raw_kept: Vec<serde_json::Value> = result.raw["transactions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .zip(kept.iter())
                    .filter(|(_, k)| **k)
                    .map(|(v, _)| v.clone())
                    .collect()
            })
            .unwrap_or_default();
        return output::print_json(&serde_json::json!({ "transactions": raw_kept }));
    }

    let rows = result
        .parsed
        .transactions
        .iter()
        .zip(kept.iter())
        .filter(|(_, k)| **k)
        .map(|(t, _)| {
            vec![
                t.date.clone(),
                t.account_name.clone().unwrap_or_else(|| "-".to_string()),
                t.payee_name.clone().unwrap_or_else(|| "-".to_string()),
                t.category_name.clone().unwrap_or_else(|| "-".to_string()),
                truncate_memo(&t.memo),
                output::milliunits(t.amount),
            ]
        })
        .collect();
    println!(
        "{}",
        output::render_table(
            &["Date", "Account", "Payee", "Category", "Memo", "Amount"],
            rows
        )
    );
    Ok(())
}
```

`src/cli/mod.rs`: add `pub mod transactions;`, the variant:

```rust
    /// List transactions
    Transactions {
        #[command(subcommand)]
        command: TransactionsCommand,
    },
```

```rust
#[derive(Debug, Subcommand)]
pub enum TransactionsCommand {
    /// List transactions (filters combine with AND)
    List {
        /// Only transactions on or after this ISO date (sent to the API)
        #[arg(long, value_name = "YYYY-MM-DD")]
        since: Option<String>,
        /// Only transactions on or before this ISO date
        #[arg(long, value_name = "YYYY-MM-DD")]
        until: Option<String>,
        /// Payee id, or case-insensitive name substring
        #[arg(long)]
        payee: Option<String>,
        /// Account id, or case-insensitive name substring
        #[arg(long)]
        account: Option<String>,
        /// Category id, or case-insensitive name substring
        #[arg(long)]
        category: Option<String>,
        /// Only transactions with no category
        #[arg(long)]
        uncategorized: bool,
        /// Only unapproved transactions
        #[arg(long)]
        unapproved: bool,
    },
}
```

and the dispatch arm:

```rust
        Command::Transactions { command } => match command {
            TransactionsCommand::List {
                since,
                until,
                payee,
                account,
                category,
                uncategorized,
                unapproved,
            } => {
                let ctx = context::build_ctx(json, budget.as_deref())?;
                transactions::list(
                    &ctx,
                    transactions::Filters {
                        since,
                        until,
                        payee,
                        account,
                        category,
                        uncategorized,
                        unapproved,
                    },
                )
                .await
            }
        },
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS (everything).

- [ ] **Step 6: Milestone gate**

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`. Fix only what your task touched; report anything else.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: transactions list with client-side filters"
```

---

## M2 carry-overs (from final review, 2026-08-02 — fold into later plans)

- Document `YNAB_PAT` in CLAUDE.md/README once the user confirms it at the checkpoint (implemented + tested, not yet user-facing-documented).
- Budget id is interpolated raw into URL paths — add percent-encoding or an early "must be a UUID or 'last-used'" validation for cleaner errors (M3 touches budget refs anyway).
- Add one cheap `accounts list` e2e with a deleted-category-in-live-group payload (closes two coverage gaps at once).
- `--json`/`--budget` globals appear in `auth`/`config` help where they're ignored — cosmetic clap polish, someday.
- `token_prefers_keychain_when_no_env` unit test fails if the dev shell exports YNAB_PAT — guard or scope it.
- DRY on the five list commands: deliberately NOT refactored (28-72 lines each, shared logic already extracted); revisit after M3 reshapes them.

## Carry-overs NOT in this plan (deliberate)

- Unused-dep pruning (`oauth2`, `anyhow`) waits for M4 (oauth2 becomes used; anyhow decision then).
- zeroize on token intermediates — M4 auth polish.
- CI grep guard for write verbs — when CI lands (distribution work).
- `Error::Input` variant — revisit only if M3 adds more user-input errors.
