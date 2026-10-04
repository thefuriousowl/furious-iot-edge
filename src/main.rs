use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    thread::sleep,
    time::Duration,
};

const MB_TCP_PID: u16 = 0;
const MBAP_HEADER_LEN: usize = 7;

pub(crate) struct ModbusOperation {
    unit_id: u8,
    function_code: FunctionCode,
    starting_address: u16,
    quantity: u16,
}

impl ModbusOperation {
    pub(crate) fn new(
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
pub(crate) struct ModbusSession {
    stream: TcpStream,
    next_transaction_id: u16,
}

impl ModbusSession {
    pub(crate) fn connect(socket_address: &str) -> Result<Self, std::io::Error> {
        let parsed_address = socket_address
            .parse::<SocketAddr>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let stream = TcpStream::connect(parsed_address)?;
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
    ) -> Result<ModbusResponse, std::io::Error> {
        if response_pdu.len() < 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Pdu Truncated",
            ));
        }
        let response_byte_count = response_pdu[1];
        if (response_byte_count as u16) != request_quantity * 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Byte count mismatch",
            ));
        }
        let data_len = pdu_len - 2;
        if (response_byte_count as usize) != data_len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Byte count mismatch",
            ));
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

    fn process_exception(response_pdu: &[u8]) -> Result<ModbusResponse, std::io::Error> {
        if response_pdu.len() != 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Pdu Truncated",
            ));
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
    ) -> Result<ModbusResponse, std::io::Error> {
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
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid MBAP length field",
            ));
        }

        let pdu_len = usize::from(res_length - 1);
        if pdu_len < 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PDU Truncated",
            ));
        }

        let mut res_pdu = [0u8; 253];
        // Read FC
        self.stream.read_exact(&mut res_pdu[..pdu_len])?;
        let res_fc = res_pdu[0];

        let res_tid = u16::from_be_bytes([res_mbap[0], res_mbap[1]]);
        if res_tid != req_tid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response transaction id mismatch from request transaction id",
            ));
        }
        let res_pid = u16::from_be_bytes([res_mbap[2], res_mbap[3]]);
        if res_pid != MB_TCP_PID {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid response protocol id",
            ));
        }

        let res_uid = res_mbap[6];
        if req_uid != res_uid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response unit id mismatch from request unit id",
            ));
        }
        if res_fc != req_pdu[0] && res_fc != (req_pdu[0] | 0x80) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Function code mismatch. expected: {}, got: {}",
                    req_pdu[0], res_fc
                ),
            ));
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
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unknown FC",
            )),
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut session = ModbusSession::connect("127.0.0.1:10502").expect("good");
    let operation_fc03 =
        ModbusOperation::new(0x01, FunctionCode::ReadHoldingRegisters, 0x0000, 0x0002)
            .expect("should be valid");
    let operation_fc04 =
        ModbusOperation::new(0x02, FunctionCode::ReadInputRegisters, 0x0000, 0x0002)
            .expect("should be valid");
    loop {
        let response_fc03 = session.execute(&operation_fc03)?;

        if let Some(registers_fc03) = response_fc03.registers() {
            println!("Registers FC03: {registers_fc03:?}");
        } else if let Some(exception_fc03) = response_fc03.exception() {
            println!("Modbus exception: {exception_fc03:?}");
        }

        let response_fc04 = session.execute(&operation_fc04)?;

        if let Some(registers_fc04) = response_fc04.registers() {
            println!("Registers FC04: {registers_fc04:?}");
        } else if let Some(exception_fc04) = response_fc04.exception() {
            println!("Modbus exception: {exception_fc04:?}");
        }

        sleep(Duration::from_secs(1));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ModbusException {
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
pub(crate) enum ModbusResponse {
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
pub(crate) enum FunctionCode {
    ReadHoldingRegisters = 0x03,
    ReadInputRegisters = 0x04,
}

#[cfg(test)]
mod tests;
