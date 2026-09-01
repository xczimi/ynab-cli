use crate::error::{Error, Result};
use comfy_table::{Table, presets};
use std::io::Write;

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

/// The ONLY path to stdout in this codebase. `println!` panics when the
/// reader hangs up (`ynab ... | head`); this reports it as an error the
/// caller can classify instead.
pub fn print_line(text: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{text}").map_err(classify)?;
    out.flush().map_err(classify)
}

fn classify(e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::BrokenPipe => Error::BrokenPipe,
        _ => Error::Io(e),
    }
}

pub fn print_json(value: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|e| Error::Decode(e.to_string()))?;
    print_line(&text)
}

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
