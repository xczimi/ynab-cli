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
