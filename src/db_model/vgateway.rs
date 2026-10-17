use rusqlite::{Connection, OptionalExtension, Row};
use serde_json::Value;

#[derive(Debug)]
pub struct VGatewayRow {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub config: Value,
    pub enabled: bool,
}
impl VGatewayRow {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            driver: row.get("driver")?,
            config: row.get("config")?,
            enabled: row.get("enabled")?,
        })
    }

    pub fn find_all(conn: &Connection) -> rusqlite::Result<Vec<VGatewayRow>> {
        let mut stmt = conn.prepare("SELECT id, name, driver, config, enabled FROM vgateway")?;
        stmt.query_map([], VGatewayRow::from_row)?.collect()
    }

    pub fn find_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<VGatewayRow>> {
        let mut stmt =
            conn.prepare("SELECT id, name, driver, config, enabled FROM vgateway WHERE id = ?1")?;
        stmt.query_row([id], VGatewayRow::from_row).optional()
    }

    pub fn find_by_name(conn: &Connection, name: &str) -> rusqlite::Result<Option<VGatewayRow>> {
        let mut stmt =
            conn.prepare("SELECT id, name, driver, config, enabled FROM vgateway WHERE name = ?1")?;
        stmt.query_row([name], VGatewayRow::from_row).optional()
    }
}
