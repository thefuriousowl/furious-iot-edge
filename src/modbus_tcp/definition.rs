use crate::modbus_tcp::MultipleWriteValues::{Coils, Registers};

pub struct ReadRequestDefinition {
    pub unit_id: u8,
    pub function_code: u8,
    pub starting_address: u16,
    pub quantity: u16,
}

pub enum SingleWriteValue {
    Coil(bool),
    Register(u16),
}

pub struct SingleWriteRequestDefinition {
    pub unit_id: u8,
    pub starting_address: u16,
    pub value: SingleWriteValue,
}

pub enum MultipleWriteValues {
    Coils(Vec<bool>),
    Registers(Vec<u16>),
}

pub struct MultipleWriteRequestDefinition {
    pub unit_id: u8,
    pub starting_address: u16,
    pub values: MultipleWriteValues,
}
