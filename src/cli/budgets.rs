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
