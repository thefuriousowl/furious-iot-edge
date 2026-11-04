use std::fmt::Display;

#[derive(Debug)]
pub enum ModbusTcpError {
    Io(std::io::Error),
    Request(RequestError),
    Sync(SyncError),
    Response(ResponseError),
    Exception(ModbusException),
}
impl ModbusTcpError {
    pub fn is_connection_reusable(&self) -> bool {
        match self {
            Self::Sync(_) | Self::Io(_) => false,
            Self::Exception(_) | Self::Response(_) | Self::Request(_) => true,
        }
    }
}
impl Display for ModbusTcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(_) => write!(f, "request validation error"),
            Self::Response(_) => write!(f, "response validation error"),
            Self::Sync(_) => write!(f, "connection out of sync"),
            Self::Exception(_) => write!(f, "modbus exception"),
            Self::Io(_) => write!(f, "transport I/O failed"),
        }
    }
}
impl std::error::Error for ModbusTcpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Exception(e) => Some(e),
            Self::Request(e) => Some(e),
            Self::Response(e) => Some(e),
            Self::Sync(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}
impl From<std::io::Error> for ModbusTcpError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<RequestError> for ModbusTcpError {
    fn from(e: RequestError) -> Self {
        Self::Request(e)
    }
}
impl From<SyncError> for ModbusTcpError {
    fn from(e: SyncError) -> Self {
        Self::Sync(e)
    }
}
impl From<ResponseError> for ModbusTcpError {
    fn from(e: ResponseError) -> Self {
        Self::Response(e)
    }
}
impl From<ModbusException> for ModbusTcpError {
    fn from(e: ModbusException) -> Self {
        Self::Exception(e)
    }
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
impl ModbusException {
    pub fn code(&self) -> u8 {
        match self {
            Self::IllegalFunction => 0x01,
            Self::IllegalDataAddress => 0x02,
            Self::IllegalDataValue => 0x03,
            Self::ServerDeviceFailure => 0x04,
            Self::Acknowledge => 0x05,
            Self::ServerDeviceBusy => 0x06,
            Self::MemoryParityError => 0x08,
            Self::GatewayPathUnavailable => 0x0a,
            Self::GatewayTargetDeviceFailedToRespond => 0x0b,
            Self::UnexpectedException { found } => *found,
        }
    }
}
impl std::error::Error for ModbusException {}

#[derive(Debug)]
#[non_exhaustive]
pub enum RequestError {
    AddressOutOfRange { min: u16, max: u16, found: u32 },
    InvalidQuantity { min: u16, max: u16, found: u16 },
    UnexpectedFunctionCode { found: u8 },
}
impl Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddressOutOfRange { min, max, found } => write!(
                f,
                "address out of range: expected {min}..={max}, found {found}"
            ),
            Self::InvalidQuantity { min, max, found } => {
                write!(f, "invalid quantity: expected {min}..={max}, found {found}")
            }
            Self::UnexpectedFunctionCode { found } => {
                write!(f, "unexpected function code: found {found:#04x}")
            }
        }
    }
}
impl std::error::Error for RequestError {}

#[derive(Debug)]
#[non_exhaustive]
pub enum ResponseError {
    InvalidProtocolId { expected: u16, found: u16 },
    UnitIdMismatch { expected: u8, found: u8 },
    FunctionCodeMismatch { request: u8, response: u8 },
    ByteCountMismatched { expected: u16, found: u16 },
    ActualDataLenMismatchByteCount { byte_count: usize, data_len: usize },
    InvalidExceptionPduLen { expected: usize, found: usize },
    InvalidWriteResponsePduLen { expected: usize, found: usize },
    WriteResponseMismatch { expected: [u8; 5], found: [u8; 5] },
}
impl Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
impl std::error::Error for ResponseError {}

#[derive(Debug)]
#[non_exhaustive]
pub enum SyncError {
    InvalidMbapLength { min: u16, max: u16, found: u16 },
    TransactionIdMismatch { expected: u16, found: u16 },
}
impl Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMbapLength { min, max, found } => write!(
                f,
                "invalid MBAP length: expected {min}..={max}, found {found}"
            ),
            Self::TransactionIdMismatch { expected, found } => write!(
                f,
                "transaction ID mismatch: expected {expected}, found {found}"
            ),
        }
    }
}
impl std::error::Error for SyncError {}
