use std::{net::IpAddr, time::Duration};

use rusqlite::Connection;
use serde::Deserialize;

use crate::{
    command::{ModbusTcpCommand, ModbusTcpEdgeOperationCommand},
    db_model::{device::DeviceRow, edge_operation::EdgeOperationRow, vgateway::VGatewayRow},
    modbus_tcp::FunctionCode,
    scheduler::MIN_ALLOW_INTERVAL,
};

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
    Disabled {
        table: &'static str,
        id: String,
    },
    UnsupportedOperation {
        table: &'static str,
        function: u8,
        name: String,
        id: String,
    },
}
impl From<rusqlite::Error> for PrepareError {
    fn from(e: rusqlite::Error) -> Self {
        PrepareError::Db(e)
    }
}
#[derive(Debug, Deserialize)]
pub struct ModbusTcpDeviceConfig {
    pub unit_id: u8,
}
#[derive(Debug, Deserialize)]
pub struct ModbusTcpEdgeOperationConfig {
    function: u8,
    address: u16,
    quantity: u16,
    poll_interval_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct ModbusTcpVGatewayConfig {
    host: IpAddr,
    port: u16,
    request_timeout_ms: u64,
}
pub fn prepare_modbus_tcp(
    conn: &Connection,
    gateway_id: &str,
) -> Result<(ModbusTcpCommand, Vec<PrepareError>), PrepareError> {
    let vgateway_record_result = VGatewayRow::find_by_id(conn, gateway_id)?;

    let vgateway_record = vgateway_record_result.ok_or_else(|| PrepareError::NotFound {
        table: "vgateway",
        key: gateway_id.to_string(),
    })?;

    if !vgateway_record.enabled {
        return Err(PrepareError::Disabled {
            table: "vgateway",
            id: gateway_id.to_string(),
        });
    }

    if vgateway_record.driver != "modbus_tcp" {
        return Err(PrepareError::InvalidConfig {
            table: "vgateway",
            id: vgateway_record.id,
            reason: "this vgateway driver is not modbus_tcp".to_string(),
        });
    }

    let parse_vgateway_config =
        serde_json::from_value::<ModbusTcpVGatewayConfig>(vgateway_record.config).map_err(|e| {
            PrepareError::InvalidConfig {
                table: "vgateway",
                id: vgateway_record.id.clone(),
                reason: e.to_string(),
            }
        })?;
    let socket_address =
        std::net::SocketAddr::new(parse_vgateway_config.host, parse_vgateway_config.port);

    if parse_vgateway_config.request_timeout_ms == 0 {
        return Err(PrepareError::InvalidConfig {
            table: "vgateway",
            id: vgateway_record.id,
            reason: format!(
                "invalid request_timeout_ms. Expect > 0 found: {}",
                parse_vgateway_config.request_timeout_ms
            ),
        });
    }

    let child_devices_record = DeviceRow::find_all_by_gateway_id(conn, gateway_id, true)?;
    let mut err_collection = Vec::new();
    let mut edge_operation_commands = Vec::new();

    for dev in child_devices_record {
        let dev_config = match serde_json::from_value::<ModbusTcpDeviceConfig>(dev.config.clone()) {
            Ok(cfg) => cfg,
            Err(e) => {
                err_collection.push(PrepareError::InvalidConfig {
                    table: "device",
                    id: dev.id,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        let child_operations_record = EdgeOperationRow::find_all_by_device_id(conn, &dev.id, true)?;

        for op in child_operations_record {
            let op_config =
                match serde_json::from_value::<ModbusTcpEdgeOperationConfig>(op.config.clone()) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        err_collection.push(PrepareError::InvalidConfig {
                            table: "edge_operation",
                            id: op.id,
                            reason: e.to_string(),
                        });
                        continue;
                    }
                };

            let (parse_fc, max_quantity) = match op_config.function {
                3 => (FunctionCode::ReadHoldingRegisters, 125),
                4 => (FunctionCode::ReadInputRegisters, 125),
                _ => {
                    err_collection.push(PrepareError::UnsupportedOperation {
                        table: "edge_operation",
                        name: op.name,
                        function: op_config.function,
                        id: op.id,
                    });
                    continue;
                }
            };

            if let Err(e) = validate_modbus_tcp_operation_config(
                &op.id,
                op_config.quantity,
                max_quantity,
                op_config.address,
                op_config.poll_interval_ms,
            ) {
                err_collection.push(e);
                continue;
            }

            edge_operation_commands.push(ModbusTcpEdgeOperationCommand {
                operation_id: op.id,
                unit_id: dev_config.unit_id,
                function_code: parse_fc,
                starting_address: op_config.address,
                quantity: op_config.quantity,
                poll_interval: Duration::from_millis(op_config.poll_interval_ms),
            });
        }
    }

    Ok((
        ModbusTcpCommand {
            gateway_id: vgateway_record.id,
            address: socket_address,
            request_timeout: Duration::from_millis(parse_vgateway_config.request_timeout_ms),
            operations: edge_operation_commands,
        },
        err_collection,
    ))
}

fn validate_modbus_tcp_operation_config(
    id: &str,
    quantity: u16,
    max_qty: u16,
    address: u16,
    poll_interval_ms: u64,
) -> Result<(), PrepareError> {
    //validate quantity
    if !(1..=max_qty).contains(&quantity) {
        return Err(PrepareError::InvalidConfig {
            table: "edge_operation",
            id: id.to_string(),
            reason: format!(
                "Invalid quantity. Expected 1-{} but found {}",
                max_qty, quantity
            ),
        });
    }
    // validate starting address + quantity in acceptable range
    if u32::from(quantity) + u32::from(address) > 65536 {
        return Err(PrepareError::InvalidConfig {
            table: "edge_operation",
            id: id.to_string(),
            reason: format!(
                "Invalid address range. Expected <= 65536 but found {}",
                u32::from(quantity) + u32::from(address)
            ),
        });
    }

    // validate poll_interval_ms must be at least 100 ms
    if Duration::from_millis(poll_interval_ms) < MIN_ALLOW_INTERVAL {
        return Err(PrepareError::InvalidConfig {
            table: "edge_operation",
            id: id.to_string(),
            reason: format!(
                "Invalid poll_interval_ms. Expected >= {:?} but found {} ms",
                MIN_ALLOW_INTERVAL, poll_interval_ms
            ),
        });
    }

    Ok(())
}
