use std::{net::SocketAddr, time::Duration};

use crate::modbus_tcp::FunctionCode;

#[derive(Debug)]
pub struct ModbusTcpCommand {
    pub gateway_id: String,
    pub address: SocketAddr,
    pub request_timeout: Duration,
    pub operations: Vec<ModbusTcpEdgeOperationCommand>,
}

#[derive(Debug, PartialEq)]
pub struct ModbusTcpEdgeOperationCommand {
    pub operation_id: String,
    pub unit_id: u8,
    pub function_code: FunctionCode,
    pub starting_address: u16,
    pub quantity: u16,
    pub poll_interval: Duration,
}
