use crate::error::ModbusTcpError;

// Validation helper
pub fn ensure(condition: bool, error: impl Into<ModbusTcpError>) -> Result<(), ModbusTcpError> {
    condition.then_some(()).ok_or(error.into())
}
