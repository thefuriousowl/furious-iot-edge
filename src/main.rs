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
    function_code: u8,
    starting_address: u16,
    quantity: u16,
}

impl ModbusOperation {
    pub(crate) fn new(
        unit_id: u8,
        function_code: u8,
        starting_address: u16,
        quantity: u16,
    ) -> Result<Self, String> {
        if function_code != 0x03 {
            return Err("Only FC03 supported".to_string());
        }

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

    pub(crate) fn function_code(&self) -> u8 {
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

    fn process_fc03_response(
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
        req_pdu[0] = operation.function_code();
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

        match res_fc {
            0x03 => {
                let modbus_response = Self::process_fc03_response(
                    &res_pdu[..pdu_len],
                    operation.quantity(),
                    pdu_len,
                )?;
                Ok(modbus_response)
            }
            0x83 => Self::process_exception(&res_pdu[..pdu_len]),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unknown FC",
            )),
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut session = ModbusSession::connect("127.0.0.1:10502").expect("good");
    let operation = ModbusOperation::new(0x01, 0x03, 0x0000, 0x0002).expect("should be valid");
    loop {
        let response = session.execute(&operation)?;

        if let Some(registers) = response.registers() {
            println!("Registers: {registers:?}");
        } else if let Some(exception) = response.exception() {
            println!("Modbus exception: {exception:?}");
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    use crate::{ModbusException, ModbusOperation, ModbusSession};
    #[test]
    fn valid_fc03_round_trip() {
        let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
        let server_addr = server.local_addr().expect("should get local address");
        // 12-byte Modbus TCP Request ADU (FC03 Read Holding Registers)
        // Transaction ID (2B), Protocol ID (2B), Length (2B), Unit ID (1B), FC (1B), Start Addr (2B), Quantity (2B)
        let expected_request_adu: [u8; 12] = [
            0x00, 0x00, // Transaction ID: 0
            0x00, 0x00, // Protocol ID: 0 (Modbus TCP)
            0x00, 0x06, // Length: 6 bytes following
            0x01, // Unit ID: 1
            0x03, // Function Code: 03 (Read Holding Registers)
            0x00, 0x00, // Starting Address: 0
            0x00, 0x02, // Quantity of Registers: 2
        ];

        // Modbus TCP Response ADU (2 registers = 4 bytes response data)
        // Header (7B) + FC (1B) + Byte Count (1B) + Register Data (4B) = 13 bytes
        let response_adu: [u8; 13] = [
            0x00, 0x00, // Transaction ID: 0 (Must be matched the Request)
            0x00, 0x00, // Protocol ID: 0
            0x00, 0x07, // Length: 7 bytes following (1 Unit ID + 1 FC + 1 ByteCount + 4 Data)
            0x01, // Unit ID: 1
            0x03, // Function Code: 03
            0x04, // Byte Count: 4 bytes (2 registers * 2 bytes)
            0x00, 0x0a, // Register 0 value: 10
            0x00, 0x14, // Register 1 value: 20
        ];
        let server_thread = std::thread::spawn(move || {
            let (mut server_stream, _) = server.accept().expect("should be ok");
            // Read exact 12-byte request
            let mut buf = [0u8; 12];
            server_stream
                .read_exact(&mut buf)
                .expect("server should read 12-byte request");

            // Assert request bytes
            assert_eq!(
                buf, expected_request_adu,
                "Request ADU does not match expected bytes"
            );

            // Write valid FC03 response
            server_stream
                .write_all(&response_adu)
                .expect("server should write response");
        });

        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("client should connect");
        let operation = ModbusOperation::new(0x01, 0x03, 0x0000, 0x0002).expect("valid operation");
        let modbus_response = session.execute(&operation).expect("should succeed");
        assert_eq!(modbus_response.registers(), Some(&[10u16, 20][..]));

        server_thread
            .join()
            .expect("server thread panicked or failed");
    }

    #[test]
    fn exception_then_valid_fc03_on_same_connection() {
        let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
        let server_addr = server.local_addr().expect("should get local address");

        let server_thread = std::thread::spawn(move || {
            let (mut server_stream, _) = server.accept().expect("server should accept connection");
            let mut expected_request = [
                0x00, 0x00, // TID: 0
                0x00, 0x00, // PID: 0
                0x00, 0x06, // MBAP Length
                0x01, // Unit ID
                0x03, // FC03
                0x00, 0x00, // Starting address
                0x00, 0x02, // Quantity
            ];
            let mut request = [0u8; 12];

            // First Transaction: read request then response the exception
            server_stream
                .read_exact(&mut request)
                .expect("server should read first request");
            assert_eq!(request, expected_request);

            let exception_response_adu = [
                0x00, 0x00, // TID: 0
                0x00, 0x00, // PID: 0
                0x00, 0x03, // Unit ID + exception PDU
                0x01, // Unit ID
                0x83, 0x02, // FC03 exception: Illegal Data Address
            ];

            server_stream
                .write_all(&exception_response_adu)
                .expect("server should write exception response");

            // Next transaction: waiting request on the same stream
            expected_request[1] = 0x01; // TID change to 1
            server_stream
                .read_exact(&mut request)
                .expect("server should read second request");
            assert_eq!(request, expected_request);

            let normal_response_adu = [
                0x00, 0x01, // TID: 1
                0x00, 0x00, // PID: 0
                0x00, 0x07, // MBAP Length
                0x01, // Unit ID
                0x03, 0x04, // FC03 + Byte Count
                0x00, 0x0a, // Register: 10
                0x00, 0x14, // Register: 20
            ];
            server_stream
                .write_all(&normal_response_adu)
                .expect("server should write normal response");
        });
        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("client should connect");
        let operation = ModbusOperation::new(0x01, 0x03, 0x0000, 0x0002).expect("valid operation");
        let response = session
            .execute(&operation)
            .expect("valid exception should be a protocol outcome");

        assert_eq!(
            response.exception(),
            Some(ModbusException::IllegalDataAddress)
        );

        let response = session
            .execute(&operation)
            .expect("second transaction should succeed");

        assert_eq!(response.registers(), Some(&[10u16, 20][..]));

        server_thread
            .join()
            .expect("server thread panicked or failed");
    }
}
