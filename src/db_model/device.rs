use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

#[derive(Debug)]
pub struct DeviceRow {
    pub id: String,
    pub gateway_id: String,
    pub name: String,
    pub slug: String,
    pub config: Value,
    pub enabled: bool,
}
impl DeviceRow {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            gateway_id: row.get("gateway_id")?,
            name: row.get("name")?,
            slug: row.get("slug")?,
            config: row.get("config")?,
            enabled: row.get("enabled")?,
        })
    }

    pub fn find_all(conn: &Connection) -> rusqlite::Result<Vec<DeviceRow>> {
        let mut stmt =
            conn.prepare("SELECT id, gateway_id, name, slug, config, enabled FROM device")?;
        stmt.query_map([], DeviceRow::from_row)?.collect()
    }

    pub fn find_all_by_gateway_id(
        conn: &Connection,
        gateway_id: &str,
        enabled: bool,
    ) -> rusqlite::Result<Vec<DeviceRow>> {
        let mut stmt = conn.prepare("SELECT id, gateway_id, name, slug, config, enabled FROM device WHERE gateway_id = ?1 AND enabled = ?2")?;
        stmt.query_map(params![gateway_id, enabled], DeviceRow::from_row)?
            .collect()
    }

    pub fn find_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<DeviceRow>> {
        let mut stmt = conn.prepare(
            "SELECT id, gateway_id, name, slug, config, enabled FROM device WHERE id = ?1",
        )?;
        stmt.query_row([id], DeviceRow::from_row).optional()
    }

    pub fn find_by_name(
        conn: &Connection,
        gateway_id: &str,
        name: &str,
    ) -> rusqlite::Result<Option<DeviceRow>> {
        let mut stmt = conn.prepare(
            "SELECT id, gateway_id, name, slug, config, enabled FROM device WHERE gateway_id = ?1 AND name = ?2 COLLATE NOCASE",
        )?;
        stmt.query_row([gateway_id, name], DeviceRow::from_row)
            .optional()
    }
}
