use std::{net::IpAddr, time::Duration};

use rusqlite::Connection;
use serde::Deserialize;

use crate::{command::ModbusTcpCommand, db_model::vgateway::VGatewayRow};

#[derive(Debug)]
pub enum PrepareError {
    Db(rusqlite::Error),
    NotFound {
        table: &'static str,
        key: String,
    },
    InvalidConfig {
        table: &'static str,
        id: String,
        reason: String,
    },
}
impl From<rusqlite::Error> for PrepareError {
    fn from(e: rusqlite::Error) -> Self {
        PrepareError::Db(e)
    }
}

#[derive(Debug, Deserialize)]
pub struct ModbusTcpConfig {
    host: IpAddr,
    port: u16,
    request_timeout_ms: u64,
    poll_interval_ms: u64,
}
pub fn prepare_modbus_tcp(
    conn: &Connection,
    gateway_id: &str,
) -> Result<ModbusTcpCommand, PrepareError> {
    let vgateway_record_result = VGatewayRow::find_by_id(conn, gateway_id)?;
    let vgateway_record = vgateway_record_result.ok_or_else(|| PrepareError::NotFound {
        table: "vgateway",
        key: gateway_id.to_string(),
    })?;

    if vgateway_record.driver != "modbus_tcp" {
        return Err(PrepareError::InvalidConfig {
            table: "vgateway",
            id: vgateway_record.id,
            reason: "this vgateway driver is not modbus_tcp".to_string(),
        });
    }

    let parse_config =
        serde_json::from_value::<ModbusTcpConfig>(vgateway_record.config).map_err(|e| {
            PrepareError::InvalidConfig {
                table: "vgateway",
                id: vgateway_record.id.clone(),
                reason: e.to_string(),
            }
        })?;
    let socket_address = std::net::SocketAddr::new(parse_config.host, parse_config.port);

    Ok(ModbusTcpCommand {
        gateway_id: vgateway_record.id,
        address: socket_address,
        request_timeout_ms: Duration::from_millis(parse_config.request_timeout_ms),
        operations: Vec::new(),
    })
}
