use crate::modbus_tcp::error::{ModbusTcpError, ValidationError};

// Validation helper
pub fn ensure(condition: bool, error: ValidationError) -> Result<(), ModbusTcpError> {
    condition
        .then_some(())
        .ok_or(ModbusTcpError::Validation(error))
}
