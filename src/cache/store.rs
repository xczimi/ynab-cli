use rusqlite::params;

use crate::cache::Cache;
use crate::error::{Error, Result};

fn db_err(e: rusqlite::Error) -> Error {
    Error::Cache(e.to_string())
}

impl Cache {
    pub fn server_knowledge(&self, budget: &str, resource: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT server_knowledge FROM sync_state
                 WHERE budget_id = ?1 AND resource = ?2",
            )
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![budget, resource], |row| row.get::<_, i64>(0))
            .map_err(db_err)?;
        match rows.next() {
            Some(v) => Ok(Some(v.map_err(db_err)?)),
            None => Ok(None),
        }
    }

    pub fn set_server_knowledge(&self, budget: &str, resource: &str, sk: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_state (budget_id, resource, server_knowledge)
                 VALUES (?1, ?2, ?3)",
                params![budget, resource, sk],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn upsert_entities(
        &mut self,
        budget: &str,
        resource: &str,
        items: &[(String, serde_json::Value)],
    ) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_err)?;
        for (id, value) in items {
            let text = serde_json::to_string(value).map_err(|e| Error::Cache(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO entities (budget_id, resource, id, json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![budget, resource, id, text],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    pub fn replace_entities(
        &mut self,
        budget: &str,
        resource: &str,
        items: &[(String, serde_json::Value)],
    ) -> Result<()> {
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute(
            "DELETE FROM entities WHERE budget_id = ?1 AND resource = ?2",
            params![budget, resource],
        )
        .map_err(db_err)?;
        for (id, value) in items {
            let text = serde_json::to_string(value).map_err(|e| Error::Cache(e.to_string()))?;
            tx.execute(
                "INSERT INTO entities (budget_id, resource, id, json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![budget, resource, id, text],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    /// `order_json_field`, when present, is interpolated directly into the SQL
    /// string (json_extract path). It is ALWAYS a compile-time constant
    /// supplied by our own sync layer (e.g. "$.date"), never user input.
    pub fn load_entities(
        &self,
        budget: &str,
        resource: &str,
        order_json_field: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        let sql = match order_json_field {
            Some(field) => format!(
                "SELECT json FROM entities WHERE budget_id = ?1 AND resource = ?2
                 ORDER BY json_extract(json, '{field}'), id"
            ),
            None => "SELECT json FROM entities WHERE budget_id = ?1 AND resource = ?2
                     ORDER BY rowid"
                .to_string(),
        };
        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(params![budget, resource], |row| row.get::<_, String>(0))
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let text = row.map_err(db_err)?;
            out.push(serde_json::from_str(&text).map_err(|e| Error::Cache(e.to_string()))?);
        }
        Ok(out)
    }

    pub fn status_rows(&self) -> Result<Vec<(String, String, i64, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.budget_id, s.resource, s.server_knowledge,
                        (SELECT count(*) FROM entities e
                          WHERE e.budget_id = s.budget_id AND e.resource = s.resource)
                 FROM sync_state s ORDER BY s.budget_id, s.resource",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
        Ok(out)
    }
}
