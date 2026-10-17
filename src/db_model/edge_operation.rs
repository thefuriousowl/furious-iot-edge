use rusqlite::{Connection, OptionalExtension, Row};
use serde_json::Value;

#[derive(Debug)]
pub struct EdgeOperationRow {
    id: String,
    device_id: String,
    name: String,
    config: Value,
    enabled: bool,
}
impl EdgeOperationRow {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            device_id: row.get("device_id")?,
            name: row.get("name")?,
            config: row.get("config")?,
            enabled: row.get("enabled")?,
        })
    }

    pub fn find_all(conn: &Connection) -> rusqlite::Result<Vec<EdgeOperationRow>> {
        let mut stmt =
            conn.prepare("SELECT id, device_id, name, config, enabled FROM edge_operation")?;
        stmt.query_map([], EdgeOperationRow::from_row)?.collect()
    }

    pub fn find_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<EdgeOperationRow>> {
        let mut stmt = conn.prepare(
            "SELECT id, device_id, name, config, enabled FROM edge_operation WHERE id = ?1",
        )?;
        stmt.query_row([id], EdgeOperationRow::from_row).optional()
    }

    pub fn find_by_name(
        conn: &Connection,
        device_id: &str,
        name: &str,
    ) -> rusqlite::Result<Option<EdgeOperationRow>> {
        let mut stmt = conn.prepare(
            "SELECT id, device_id, name, config, enabled FROM edge_operation WHERE device_id = ?1 AND name = ?2 COLLATE NOCASE",
        )?;
        stmt.query_row([device_id, name], EdgeOperationRow::from_row)
            .optional()
    }
}
