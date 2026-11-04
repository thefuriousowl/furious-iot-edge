use std::fmt::Display;

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
impl Display for ModbusTcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(_) => write!(f, "protocol validation failed"),
            Self::Exception(_) => write!(f, "modbus exception"),
            Self::Io(_) => write!(f, "transport I/O failed"),
        }
    }
}
impl std::error::Error for ModbusTcpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Exception(e) => Some(e),
            Self::Validation(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum ValidationError {
    AddressOutOfRange { min: u16, max: u16, found: u32 },
    InvalidQuantity { min: u16, max: u16, found: u16 },
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
impl Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddressOutOfRange { min, max, found } => write!(
                f,
                "address out of range: expected {min}..={max}, found {found}"
            ),
            Self::InvalidQuantity { min, max, found } => {
                write!(f, "invalid quantity: expected {min}..={max}, found {found}")
            }
            Self::InvalidMbapLength { min, max, found } => write!(
                f,
                "invalid MBAP length: expected {min}..={max}, found {found}"
            ),
            Self::TransactionIdMismatch { expected, found } => write!(
                f,
                "transaction ID mismatch: expected {expected}, found {found}"
            ),
            Self::InvalidProtocolId { expected, found } => {
                write!(f, "invalid Protocol ID: expected {expected}, found {found}")
            }
            Self::UnitIdMismatch { expected, found } => {
                write!(f, "unit ID mismatch: expected {expected}, found {found}")
            }
            Self::FunctionCodeMismatch { request, response } => write!(
                f,
                "function code mismatch: expected {request:#04x}, found {response:#04x}"
            ),
            Self::UnexpectedFunctionCode { found } => {
                write!(f, "unexpected function code: found {found:#04x}")
            }
            Self::ByteCountMismatched { expected, found } => {
                write!(f, "byte count mismatch: expected {expected}, found {found}")
            }
            Self::ActualDataLenMismatchByteCount {
                byte_count,
                data_len,
            } => write!(
                f,
                "response data len mismatch from response byte count: byte count declared {byte_count}, found actual count {data_len}"
            ),
            Self::InvalidExceptionPduLen { expected, found } => write!(
                f,
                "invalid exception pdu len: expected {expected}, found {found}"
            ),
            Self::InvalidWriteResponsePduLen { expected, found } => write!(
                f,
                "invalid write response pdu len: expected {expected}, found {found}"
            ),
            Self::WriteResponseMismatch { expected, found } => write!(
                f,
                "invalid write response mismatch: expected {expected:02x?}, found {found:02x?}"
            ),
        }
    }
}
impl std::error::Error for ValidationError {}

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
impl Display for ModbusException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IllegalFunction => write!(f, "illegal function"),
            Self::IllegalDataAddress => write!(f, "illegal data address"),
            Self::IllegalDataValue => write!(f, "illegal data value"),
            Self::ServerDeviceFailure => write!(f, "server device failure"),
            Self::Acknowledge => write!(f, "acknowledge"),
            Self::ServerDeviceBusy => write!(f, "server device busy"),
            Self::MemoryParityError => write!(f, "memory parity error"),
            Self::GatewayPathUnavailable => write!(f, "gateway path unavailable"),
            Self::GatewayTargetDeviceFailedToRespond => {
                write!(f, "gateway target device failed to respond")
            }
            Self::UnexpectedException { found } => {
                write!(f, "unexpected exception code: found {found:#04x}")
            }
        }
    }
}
impl std::error::Error for ModbusException {}
