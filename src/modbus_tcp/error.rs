#[derive(Debug)]
pub enum ModbusTcpError {
    Io(std::io::Error),
    Exception(ModbusException),
    Validation(ValidationError),
}
impl From<std::io::Error> for ModbusTcpError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug)]
pub enum ValidationError {
    AddressOutOfRange { min: u16, max: u16, found: u32 },
    InvalidQuantity { min: u16, max: u16, found: u16 },
    PduTruncated { got_len: usize },
    InvalidMbapLength { min: u16, max: u16, found: u16 },
    TransactionIdMismatch { expected: u16, found: u16 },
    InvalidProtocolId { expected: u16, found: u16 },
    UnitIdMismatch { expected: u8, found: u8 },
    FunctionCodeMismatch { request: u8, response: u8 },
    UnexpectedFunctionCode { found: u8 },
    ByteCountMismatched { expected: u16, found: u16 },
    ActualDataLenMismatchByteCount { byte_count: usize, data_len: usize },
    InvalidExceptionPduLen { expected: usize, found: usize },
    InvalidWriteResponsePduLen { expected: usize, found: usize },
    WriteResponseMismatch { expected: [u8; 5], found: [u8; 5] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ModbusException {
    IllegalFunction = 0x01,
    IllegalDataAddress = 0x02,
    IllegalDataValue = 0x03,
    ServerDeviceFailure = 0x04,
    Acknowledge = 0x05,
    ServerDeviceBusy = 0x06,
    MemoryParityError = 0x08,
    GatewayPathUnavailable = 0x0a,
    GatewayTargetDeviceFailedToRespond = 0x0b,
    UnexpectedException { found: u8 },
}
impl From<u8> for ModbusException {
    fn from(code: u8) -> Self {
        match code {
            0x01 => Self::IllegalFunction,
            0x02 => Self::IllegalDataAddress,
            0x03 => Self::IllegalDataValue,
            0x04 => Self::ServerDeviceFailure,
            0x05 => Self::Acknowledge,
            0x06 => Self::ServerDeviceBusy,
            0x08 => Self::MemoryParityError,
            0x0a => Self::GatewayPathUnavailable,
            0x0b => Self::GatewayTargetDeviceFailedToRespond,
            found => Self::UnexpectedException { found },
        }
    }
}
