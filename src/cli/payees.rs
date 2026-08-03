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
    async fn payees_list_human_skips_deleted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/payees"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "payees": [
                    { "id": "p-1", "name": "Grocer", "deleted": false },
                    { "id": "p-2", "name": "Old", "deleted": true }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        list(&ctx(&server, false)).await.unwrap();
        list(&ctx(&server, true)).await.unwrap();
    }
}
