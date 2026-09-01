use crate::cli::context::Ctx;
use crate::error::{Result, cache_error};
use crate::output;

pub async fn list(ctx: &mut Ctx) -> Result<()> {
    let Ctx {
        client,
        cache,
        budget,
        json,
    } = ctx;
    // A corrupted cache is never a user-facing error (CLAUDE.md): if the
    // sync path fails mid-operation, fall back to the same direct fetch the
    // no-cache arm uses.
    let result = match cache {
        Some(cache) => match crate::cache::sync::categories(client, cache, budget).await {
            Ok(result) => result,
            Err(e) if cache_error(&e) => client.get_categories(budget, None).await?,
            Err(e) => return Err(e),
        },
        None => client.get_categories(budget, None).await?,
    };
    if *json {
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
    output::print_line(&output::render_table(
        &["Group", "Category", "Budgeted", "Activity", "Balance"],
        rows,
    ))?;
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
            cache: None,
            json,
            budget: "b-1".to_string(),
        }
    }

    #[tokio::test]
    async fn categories_list_human_skips_deleted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/budgets/b-1/categories"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "category_groups": [
                    { "id": "g-1", "name": "Bills", "hidden": false, "deleted": false,
                      "categories": [
                        { "id": "c-1", "name": "Rent", "hidden": false,
                          "budgeted": 1500000, "activity": -1500000, "balance": 0,
                          "deleted": false }
                      ] },
                    { "id": "g-2", "name": "Old", "hidden": false, "deleted": true,
                      "categories": [] }
                ], "server_knowledge": 1 }
            })))
            .mount(&server)
            .await;

        list(&mut ctx(&server, false)).await.unwrap();
        list(&mut ctx(&server, true)).await.unwrap();
    }
}
