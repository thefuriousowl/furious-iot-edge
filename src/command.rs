use std::{net::SocketAddr, time::Duration};

use crate::modbus_tcp::FunctionCode;

#[derive(Debug)]
pub struct ModbusTcpCommand {
    pub gateway_id: String,
    pub address: SocketAddr,
    pub request_timeout_ms: Duration,
    pub operations: Vec<ModbusTcpOperationCommand>,
}

#[derive(Debug)]
pub struct ModbusTcpOperationCommand {
    pub operation_id: String,
    pub unit_id: u8,
    pub function_code: FunctionCode,
    pub starting_address: u16,
    pub quantity: u16,
    pub poll_interval: Duration,
}
