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

        list(&ctx(&server, false)).await.unwrap();
        list(&ctx(&server, true)).await.unwrap();
    }
}
