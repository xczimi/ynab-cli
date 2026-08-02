use serde::Deserialize;

/// YNAB wraps every success payload as {"data": ...}.
#[derive(Debug, Deserialize)]
pub struct DataEnvelope<T> {
    pub data: T,
}

/// YNAB wraps every error payload as {"error": {"detail": ...}}.
#[derive(Debug, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
    pub detail: String,
}

#[derive(Debug, Deserialize)]
pub struct UserWrapper {
    pub user: User,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: String,
}

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
