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

pub(crate) fn normalize_date(value: &str, flag: &str) -> Result<String> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|parsed| parsed.format("%Y-%m-%d").to_string())
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
    !t.deleted && matches_filters(t, f)
}

fn matches_filters(t: &Transaction, f: &Filters) -> bool {
    if let Some(until) = &f.until
        && t.date.as_str() > until.as_str()
    {
        return false;
    }
    if let Some(p) = &f.payee
        && !name_or_id(p, t.payee_id.as_deref(), t.payee_name.as_deref())
    {
        return false;
    }
    if let Some(a) = &f.account
        && !name_or_id(a, Some(t.account_id.as_str()), t.account_name.as_deref())
    {
        return false;
    }
    if let Some(c) = &f.category
        && !name_or_id(c, t.category_id.as_deref(), t.category_name.as_deref())
    {
        return false;
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
    let since = filters
        .since
        .as_deref()
        .map(|s| normalize_date(s, "--since"))
        .transpose()?;
    let until = filters
        .until
        .as_deref()
        .map(|u| normalize_date(u, "--until"))
        .transpose()?;
    let filters = Filters {
        since,
        until,
        ..filters
    };

    let result = ctx
        .client
        .get_transactions(&ctx.budget, filters.since.as_deref())
        .await?;

    if ctx.json {
        let matches: Vec<bool> = result
            .parsed
            .transactions
            .iter()
            .map(|t| matches_filters(t, &filters))
            .collect();
        let raw_kept: Vec<serde_json::Value> = result.raw["transactions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .zip(matches.iter())
                    .filter(|(_, k)| **k)
                    .map(|(v, _)| v.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut envelope = result.raw.clone();
        envelope["transactions"] = serde_json::Value::Array(raw_kept);
        return output::print_json(&envelope);
    }

    let kept: Vec<bool> = result
        .parsed
        .transactions
        .iter()
        .map(|t| keep(t, &filters))
        .collect();

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
        let f = Filters {
            until: Some("2026-07-15".into()),
            ..Default::default()
        };
        assert!(keep(&tx(), &f));
        let f = Filters {
            until: Some("2026-07-14".into()),
            ..Default::default()
        };
        assert!(!keep(&tx(), &f));
    }

    #[test]
    fn payee_matches_id_or_name_substring() {
        let f = Filters {
            payee: Some("p-1".into()),
            ..Default::default()
        };
        assert!(keep(&tx(), &f));
        let f = Filters {
            payee: Some("grocer".into()),
            ..Default::default()
        };
        assert!(keep(&tx(), &f));
        let f = Filters {
            payee: Some("landlord".into()),
            ..Default::default()
        };
        assert!(!keep(&tx(), &f));
    }

    #[test]
    fn uncategorized_and_unapproved() {
        let f = Filters {
            uncategorized: true,
            ..Default::default()
        };
        assert!(!keep(&tx(), &f));
        let mut t = tx();
        t.category_id = None;
        assert!(keep(&t, &f));

        let f = Filters {
            unapproved: true,
            ..Default::default()
        };
        assert!(!keep(&tx(), &f));
        let mut t = tx();
        t.approved = false;
        assert!(keep(&t, &f));
    }

    #[test]
    fn date_validation() {
        assert!(normalize_date("2026-07-01", "--since").is_ok());
        let err = normalize_date("07/01/2026", "--since").unwrap_err();
        assert_eq!(
            err.to_string(),
            "config error: --since must be an ISO date (YYYY-MM-DD)"
        );
        assert_eq!(normalize_date("2026-7-1", "--since").unwrap(), "2026-07-01");
    }

    #[test]
    fn deleted_fails_keep_but_passes_matches_filters() {
        let f = Filters::default();
        let mut dead = tx();
        dead.deleted = true;
        assert!(!keep(&dead, &f));
        assert!(matches_filters(&dead, &f));
    }
}
