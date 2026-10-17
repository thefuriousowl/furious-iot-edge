use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

const MB_TCP_PID: u16 = 0;
const MBAP_HEADER_LEN: usize = 7;

pub struct ModbusOperation {
    unit_id: u8,
    function_code: FunctionCode,
    starting_address: u16,
    quantity: u16,
}

impl ModbusOperation {
    pub fn new(
        unit_id: u8,
        function_code: FunctionCode,
        starting_address: u16,
        quantity: u16,
    ) -> Result<Self, String> {
        if !(1..=125).contains(&quantity) {
            return Err("Quantity must be 1-125".to_string());
        }

        if (quantity as usize) + (starting_address as usize) > 65536 {
            return Err("Register address range exceeds 65535".to_string());
        }

        Ok(Self {
            unit_id,
            function_code,
            starting_address,
            quantity,
        })
    }

    // Getters
    pub(crate) fn unit_id(&self) -> u8 {
        self.unit_id
    }

    pub(crate) fn function_code(&self) -> FunctionCode {
        self.function_code
    }

    pub(crate) fn starting_address(&self) -> u16 {
        self.starting_address
    }

    pub(crate) fn quantity(&self) -> u16 {
        self.quantity
    }
}

#[derive(Debug)]
pub struct ModbusSession {
    stream: TcpStream,
    next_transaction_id: u16,
}

impl ModbusSession {
    pub fn connect(socket_address: &str, read_timeout: Duration) -> Result<Self, std::io::Error> {
        let parsed_address = socket_address
            .parse::<SocketAddr>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let stream = TcpStream::connect(parsed_address)?;

        stream.set_read_timeout(Some(read_timeout))?;
        Ok(Self {
            stream,
            next_transaction_id: 0,
        })
    }

    pub(crate) fn increment_transaction_id(&mut self) {
        self.next_transaction_id = self.next_transaction_id.wrapping_add(1);
    }

    fn process_register_response(
        response_pdu: &[u8],
        request_quantity: u16,
        pdu_len: usize,
    ) -> Result<ModbusResponse, ModbusError> {
        if response_pdu.len() < 2 {
            return Err(ModbusError::PduTruncated {
                got_len: response_pdu.len(),
            });
        }
        let response_byte_count = response_pdu[1];
        if (response_byte_count as u16) != request_quantity * 2 {
            return Err(ModbusError::ByteCountMismatched {
                expected: request_quantity * 2,
                found: response_byte_count as u16,
            });
        }
        let data_len = pdu_len - 2;
        if (response_byte_count as usize) != data_len {
            return Err(ModbusError::ActualDataLenMismatchByteCount {
                byte_count: response_byte_count as usize,
                data_len,
            });
        }

        let response_data = &response_pdu[2..pdu_len];
        let mut registers = [0u16; 125];
        let (chunks, _remainder) = response_data.as_chunks::<2>();
        for (dst, chunk) in registers[..request_quantity as usize]
            .iter_mut()
            .zip(chunks)
        {
            *dst = u16::from_be_bytes(*chunk);
        }
        // println!("Registers: {:?}", registers);
        Ok(ModbusResponse::Registers {
            data: registers,
            count: request_quantity as usize,
        })
    }

    fn process_exception(response_pdu: &[u8]) -> Result<ModbusResponse, ModbusError> {
        if response_pdu.len() != 2 {
            return Err(ModbusError::InvalidExceptionPduLen {
                expected: 2,
                found: response_pdu.len(),
            });
        }
        let exception = match response_pdu[1] {
            0x01 => ModbusException::IllegalFunction,
            0x02 => ModbusException::IllegalDataAddress,
            0x03 => ModbusException::IllegalDataValue,
            0x04 => ModbusException::ServerDeviceFailure,
            0x05 => ModbusException::Acknowledge,
            0x06 => ModbusException::ServerDeviceBusy,
            0x08 => ModbusException::MemoryParityError,
            0x0a => ModbusException::GatewayPathUnavailable,
            0x0b => ModbusException::GatewayTargetDeviceFailedToRespond,
            _ => ModbusException::UnexpectedException {
                found: response_pdu[1],
            },
        };
        Ok(ModbusResponse::Exception(exception))
    }

    pub(crate) fn execute(
        &mut self,
        operation: &ModbusOperation,
    ) -> Result<ModbusResponse, ModbusError> {
        let mut req_pdu = [0u8; 5];
        req_pdu[0] = operation.function_code() as u8;
        req_pdu[1..3].copy_from_slice(&operation.starting_address().to_be_bytes());
        req_pdu[3..5].copy_from_slice(&operation.quantity().to_be_bytes());

        let mut req_mbap = [0u8; MBAP_HEADER_LEN];
        let req_mbap_length: u16 = 6;
        let req_uid = operation.unit_id();
        let req_tid = self.next_transaction_id;
        req_mbap[0..2].copy_from_slice(&req_tid.to_be_bytes());
        req_mbap[2..4].copy_from_slice(&MB_TCP_PID.to_be_bytes());
        req_mbap[4..6].copy_from_slice(&req_mbap_length.to_be_bytes());
        req_mbap[6] = req_uid;

        let mut adu = [0u8; 12];
        adu[..MBAP_HEADER_LEN].copy_from_slice(&req_mbap);
        adu[MBAP_HEADER_LEN..].copy_from_slice(&req_pdu);
        self.increment_transaction_id();
        self.stream.write_all(&adu)?;
        let mut res_mbap = [0u8; MBAP_HEADER_LEN];

        self.stream.read_exact(&mut res_mbap)?;

        let res_length = u16::from_be_bytes([res_mbap[4], res_mbap[5]]);
        if !(2..=254).contains(&res_length) {
            return Err(ModbusError::InvalidMbapLength {
                min: 2,
                max: 254,
                found: res_length,
            });
        }

        let pdu_len = usize::from(res_length - 1);
        if pdu_len < 2 {
            if pdu_len == 1 {
                let mut _trash_byte = [0u8; 1];
                self.stream.read_exact(&mut _trash_byte)?;
            }
            return Err(ModbusError::PduTruncated { got_len: pdu_len });
        }

        let mut res_pdu = [0u8; 253];
        // Read FC
        self.stream.read_exact(&mut res_pdu[..pdu_len])?;
        let res_fc = res_pdu[0];

        let res_tid = u16::from_be_bytes([res_mbap[0], res_mbap[1]]);
        if res_tid != req_tid {
            return Err(ModbusError::TransactionIdMismatch {
                expected: req_tid,
                found: res_tid,
            });
        }
        let res_pid = u16::from_be_bytes([res_mbap[2], res_mbap[3]]);
        if res_pid != MB_TCP_PID {
            return Err(ModbusError::InvalidProtocolId {
                expected: MB_TCP_PID,
                found: res_pid,
            });
        }

        let res_uid = res_mbap[6];
        if req_uid != res_uid {
            return Err(ModbusError::UnitIdMismatch {
                expected: req_uid,
                found: res_uid,
            });
        }
        if res_fc != req_pdu[0] && res_fc != (req_pdu[0] | 0x80) {
            return Err(ModbusError::FunctionCodeMismatch {
                request: req_pdu[0],
                response: res_fc,
            });
        }

        match res_fc {
            0x03 | 0x04 => {
                let modbus_response = Self::process_register_response(
                    &res_pdu[..pdu_len],
                    operation.quantity(),
                    pdu_len,
                )?;
                Ok(modbus_response)
            }

            0x83 | 0x84 => Self::process_exception(&res_pdu[..pdu_len]),
            _ => Err(ModbusError::UnexpectedFunctionCode { found: res_fc }),
        }
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

#[allow(
    clippy::large_enum_variant,
    reason = "Fixed register storage avoids per-transaction heap allocation"
)]
pub enum ModbusResponse {
    // Bits { data: [u8; 250], count: usize },
    Registers { data: [u16; 125], count: usize },
    Exception(ModbusException),
}

impl ModbusResponse {
    pub(crate) fn registers(&self) -> Option<&[u16]> {
        match self {
            Self::Registers { data, count } => Some(&data[..*count]),
            Self::Exception(_) => None,
        }
    }

    pub(crate) fn exception(&self) -> Option<ModbusException> {
        match self {
            Self::Exception(exception) => Some(*exception),
            Self::Registers { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FunctionCode {
    ReadHoldingRegisters = 0x03,
    ReadInputRegisters = 0x04,
}

#[derive(Debug)]
pub enum ModbusError {
    Io(std::io::Error),
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
}

impl ModbusError {
    pub(crate) fn connection_still_usable(&self) -> bool {
        match self {
            Self::PduTruncated { .. }
            | Self::InvalidExceptionPduLen { .. }
            | Self::ByteCountMismatched { .. }
            | Self::FunctionCodeMismatch { .. } => true,
            Self::Io(..)
            | Self::InvalidMbapLength { .. }
            | Self::TransactionIdMismatch { .. }
            | Self::InvalidProtocolId { .. }
            | Self::UnitIdMismatch { .. }
            | Self::UnexpectedFunctionCode { .. }
            | Self::ActualDataLenMismatchByteCount { .. } => false,
        }
    }
}

impl From<io::Error> for ModbusError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for ModbusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActualDataLenMismatchByteCount {
                byte_count,
                data_len,
            } => write!(
                f,
                "Actual Response data len and response byte count mismatched: expected {byte_count}, found {data_len}"
            ),
            Self::ByteCountMismatched { expected, found } => write!(
                f,
                "Response byte count mismatch from request quantity: expected {expected}, found {found}"
            ),
            Self::FunctionCodeMismatch { request, response } => write!(
                f,
                "Response function code mismatch request function code: expected {request}, found {response}"
            ),
            Self::InvalidMbapLength { min, max, found } => write!(
                f,
                "Invalid MBAP length: min {min}, max {max}, found {found}"
            ),
            Self::InvalidProtocolId { expected, found } => {
                write!(f, "Invalid protocol ID: expected {expected}, found {found}")
            }
            Self::PduTruncated { got_len } => {
                write!(f, "Response PDU truncated: got_len {got_len}")
            }
            Self::TransactionIdMismatch { expected, found } => write!(
                f,
                "Response transaction ID mismatch from request transaction ID: expected {expected}, found {found}"
            ),
            Self::UnexpectedFunctionCode { found } => {
                write!(f, "Unexpected function code: found {found}")
            }
            Self::UnitIdMismatch { expected, found } => write!(
                f,
                "Response unit ID mismatch from request unit ID: expected {expected}, found {found}"
            ),
            Self::InvalidExceptionPduLen { expected, found } => write!(
                f,
                "Invalid exception PDU len: expected {expected}, found {found}"
            ),
            Self::Io(error) => write!(f, "I/O Error: {error}"),
        }
    }
}
