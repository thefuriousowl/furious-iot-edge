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
mod tests {
    use crate::{ModbusException, ModbusOperation, ModbusSession};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    // In ADU summaries, LENGTH is the MBAP Length field (Unit ID + PDU), in bytes.
    fn create_tcp_test_server(
        server_fn: impl FnOnce(std::net::TcpStream) + Send + 'static,
    ) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
        let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
        let server_addr = server
            .local_addr()
            .expect("should get local test server address");
        let server_thread = thread::spawn(move || {
            let (stream, _) = server.accept().expect("should accept client connection");

            server_fn(stream);
        });
        (server_addr, server_thread)
    }

    fn serve_exchanges(mut stream: std::net::TcpStream, exchanges: &[([u8; 12], &[u8])]) {
        let mut request = [0u8; 12];

        for (expected_request, response) in exchanges {
            stream
                .read_exact(&mut request)
                .expect("should read request");

            assert_eq!(&request, expected_request);

            stream.write_all(response).expect("should write response");
        }
    }

    fn assert_invalid_data(
        response: std::io::Result<crate::ModbusResponse>,
        expected_message: &str,
    ) {
        match response {
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                assert!(
                    error.to_string().starts_with(expected_message),
                    "unexpected error: {error}"
                );
            }
            Ok(_) => panic!("expected InvalidData: {expected_message}"),
        }
    }
    #[test]
    fn reject_mismatch_unit_id() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_request_fc03_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 0, PID: 0, LENGTH: 7, UnitID: 2, FC: 3, ByteCount: 4, Data: [10, 20]
            let mismatch_uid_fc03_response = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x02, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            serve_exchanges(
                server_stream,
                &[(expected_request_fc03_adu, &mismatch_uid_fc03_response)],
            );
        });

        let mut session = ModbusSession::connect(&server_addr.to_string())
            .expect("should connect to test server");
        let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("valid FC03 operation");
        let response = session.execute(&operation);
        assert_invalid_data(response, "response unit id mismatch");

        server_thread.join().expect("server thread panic or failed");
    }
    #[test]
    fn reject_mismatch_protocol_id() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_request_fc03_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 0, PID: 1, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
            let mismatch_pid_fc03_response = [
                0x00, 0x00, 0x00, 0x01, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            serve_exchanges(
                server_stream,
                &[(expected_request_fc03_adu, &mismatch_pid_fc03_response)],
            );
        });

        let mut session = ModbusSession::connect(&server_addr.to_string())
            .expect("should connect to test server");
        let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("valid FC03 operation");
        let response = session.execute(&operation);
        assert_invalid_data(response, "Invalid response protocol id");

        server_thread.join().expect("server thread panic or failed");
    }
    #[test]
    fn reject_mismatch_transaction_id() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_request_fc03_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
            let mismatch_tid_fc03_response = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            serve_exchanges(
                server_stream,
                &[(expected_request_fc03_adu, &mismatch_tid_fc03_response)],
            );
        });

        let mut session = ModbusSession::connect(&server_addr.to_string())
            .expect("should connect to test server");
        let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("valid FC03 operation");
        let response = session.execute(&operation);
        assert_invalid_data(response, "response transaction id mismatch");

        server_thread.join().expect("server thread panic or failed");
    }

    #[test]
    fn reject_byte_count_mismatch_quantity() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_first_request_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // Requested quantity: 2 registers (4 data bytes); response Byte Count: 2.
            // TID: 0, PID: 0, LENGTH: 5, UnitID: 1, FC: 3, ByteCount: 2, Registers: [10]
            let mismatch_byte_count_response_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0x00, 0x0a,
            ];

            // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_second_request_adu = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // Response Byte Count: 4; actual response data: 2 bytes (1 register).
            // TID: 1, PID: 0, LENGTH: 5, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10]
            let mismatch_data_bytes_len_response_adu = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x04, 0x00, 0x0a,
            ];

            serve_exchanges(
                server_stream,
                &[
                    (
                        expected_first_request_adu,
                        &mismatch_byte_count_response_adu,
                    ),
                    (
                        expected_second_request_adu,
                        &mismatch_data_bytes_len_response_adu,
                    ),
                ],
            );
        });
        let mut session = ModbusSession::connect(&server_addr.to_string())
            .expect("should connect to test server");
        let operation_a = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("valid fc03 request");
        let response = session.execute(&operation_a);
        assert_invalid_data(response, "Byte count mismatch");

        let operation_b = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("valid fc03 request");
        let response = session.execute(&operation_b);
        assert_invalid_data(response, "Byte count mismatch");
        server_thread.join().expect("server thread panic or failed");
    }
    #[test]
    fn reject_mismatch_fc() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 4, Start: 0, Quantity: 2
            let expected_first_request = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x04, 0x00, 0x00, 0x00, 0x02,
            ];
            // request fc04 but response fc03
            // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
            let mismatch_fc03_response_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];
            // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_second_request = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // request fc03 but response fc04
            // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 4, ByteCount: 4, Registers: [10, 20]
            let mismatch_fc04_response_adu = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x01, 0x04, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];
            // TID: 2, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_third_request = [
                0x00, 0x02, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // request fc03 but response fc04 exception
            // TID: 2, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x04), ExceptionCode: 2
            let mismatch_fc04_exception_response_adu =
                [0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x01, 0x84, 0x02];

            // TID: 3, PID: 0, LENGTH: 6, UnitID: 1, FC: 4, Start: 0, Quantity: 2
            let expected_forth_request = [
                0x00, 0x03, 0x00, 0x00, 0x00, 0x06, 0x01, 0x04, 0x00, 0x00, 0x00, 0x02,
            ];
            // request fc04 but response fc03 exception
            // TID: 3, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x03), ExceptionCode: 2
            let mismatch_fc03_exception_response_adu =
                [0x00, 0x03, 0x00, 0x00, 0x00, 0x03, 0x01, 0x83, 0x02];

            serve_exchanges(
                server_stream,
                &[
                    (expected_first_request, &mismatch_fc03_response_adu),
                    (expected_second_request, &mismatch_fc04_response_adu),
                    (
                        expected_third_request,
                        &mismatch_fc04_exception_response_adu,
                    ),
                    (
                        expected_forth_request,
                        &mismatch_fc03_exception_response_adu,
                    ),
                ],
            );
        });

        let operation_a = ModbusOperation::new(1, crate::FunctionCode::ReadInputRegisters, 0, 2)
            .expect("should be valid fc04 operation");
        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("should successfully connect");
        let response = session.execute(&operation_a);
        assert_invalid_data(response, "Function code mismatch");

        let operation_b = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("should be valid fc03 operation");
        let response = session.execute(&operation_b);
        assert_invalid_data(response, "Function code mismatch");

        let operation_c = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
            .expect("should be valid fc03 operation");
        let response = session.execute(&operation_c);
        assert_invalid_data(response, "Function code mismatch");

        let operation_d = ModbusOperation::new(1, crate::FunctionCode::ReadInputRegisters, 0, 2)
            .expect("should be valid fc04 operation");
        let response = session.execute(&operation_d);
        assert_invalid_data(response, "Function code mismatch");

        server_thread
            .join()
            .expect("server thread panicked or failed");
    }
    #[test]
    fn exception_then_valid_fc04_on_same_connection() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 4, Start: 16, Quantity: 2
            let expected_first_request = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x02,
            ];

            // TID: 0, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x04), ExceptionCode: 2
            let exception_response_adu = [0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x84, 0x02];

            // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 4, Start: 16, Quantity: 2
            let expected_second_request = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x02,
            ];
            // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 4, ByteCount: 4, Registers: [10, 20]
            let valid_fc04_response_adu = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x01, 0x04, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            serve_exchanges(
                server_stream,
                &[
                    (expected_first_request, &exception_response_adu),
                    (expected_second_request, &valid_fc04_response_adu),
                ],
            );
        });

        let fc04_operation =
            ModbusOperation::new(1, crate::FunctionCode::ReadInputRegisters, 16, 2)
                .expect("should be valid response");
        let mut session = ModbusSession::connect(&server_addr.to_string())
            .expect("should connect to test server");
        let response = session
            .execute(&fc04_operation)
            .expect("should get exception");
        assert_eq!(
            response.exception(),
            Some(ModbusException::IllegalDataAddress)
        );
        assert_eq!(response.registers(), None);

        let response = session
            .execute(&fc04_operation)
            .expect("second transaction should get valid registers");
        assert_eq!(response.registers(), Some(&[10u16, 20][..]));
        assert_eq!(response.exception(), None);
        server_thread
            .join()
            .expect("server thread panicked or failed");
    }

    #[test]
    fn different_fc_operations_on_same_connection() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_request_fc03_adu = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
            let valid_fc03_response = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            // TID: 1, PID: 0, LENGTH: 6, UnitID: 2, FC: 4, Start: 0, Quantity: 2
            let expected_request_fc04_adu = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x02, 0x04, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 1, PID: 0, LENGTH: 7, UnitID: 2, FC: 4, ByteCount: 4, Registers: [30, 40]
            let valid_fc04_response = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x02, 0x04, 0x04, 0x00, 0x1e, 0x00, 0x28,
            ];

            serve_exchanges(
                server_stream,
                &[
                    (expected_request_fc03_adu, &valid_fc03_response),
                    (expected_request_fc04_adu, &valid_fc04_response),
                ],
            );
        });

        let fc03_operation =
            ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
                .expect("valid operation");
        let fc04_operation = ModbusOperation::new(2, crate::FunctionCode::ReadInputRegisters, 0, 2)
            .expect("valid operation");

        let mut session = ModbusSession::connect(&server_addr.to_string())
            .expect("should successfully connect to server");
        let fc03_registers = session
            .execute(&fc03_operation)
            .expect("valid fc03 registers");
        let fc04_registers = session
            .execute(&fc04_operation)
            .expect("valid fc04 registers");

        server_thread
            .join()
            .expect("server thread panicked or failed");

        assert_eq!(fc03_registers.registers(), Some(&[10u16, 20][..]));
        assert_eq!(fc04_registers.registers(), Some(&[30u16, 40][..]));
    }
    #[test]
    fn valid_fc03_round_trip() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_request_adu: [u8; 12] = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];

            // Modbus TCP Response ADU (2 registers = 4 bytes response data)
            // Header (7B) + FC (1B) + Byte Count (1B) + Register Data (4B) = 13 bytes
            // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
            let response_adu: [u8; 13] = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            serve_exchanges(server_stream, &[(expected_request_adu, &response_adu)]);
        });

        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("client should connect");
        let operation = ModbusOperation::new(
            0x01,
            crate::FunctionCode::ReadHoldingRegisters,
            0x0000,
            0x0002,
        )
        .expect("valid operation");
        let modbus_response = session.execute(&operation).expect("should succeed");
        assert_eq!(modbus_response.registers(), Some(&[10u16, 20][..]));

        server_thread
            .join()
            .expect("server thread panicked or failed");
    }

    #[test]
    fn exception_then_valid_fc03_on_same_connection() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_first_request = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 0, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x03), ExceptionCode: 2
            let exception_response_adu = [0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x83, 0x02];

            // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_second_request = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
            let normal_response_adu = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];

            serve_exchanges(
                server_stream,
                &[
                    (expected_first_request, &exception_response_adu),
                    (expected_second_request, &normal_response_adu),
                ],
            );
        });

        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("client should connect");
        let operation = ModbusOperation::new(
            0x01,
            crate::FunctionCode::ReadHoldingRegisters,
            0x0000,
            0x0002,
        )
        .expect("valid operation");
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

    #[test]
    fn different_fc03_operations_on_same_connection() {
        let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
            // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
            let expected_request_from_op_a = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x02,
            ];
            // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
            let expected_response_for_op_a = [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a, 0x00, 0x14,
            ];
            // TID: 1, PID: 0, LENGTH: 6, UnitID: 2, FC: 3, Start: 10, Quantity: 1
            let expected_request_from_op_b = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x02, 0x03, 0x00, 0x0a, 0x00, 0x01,
            ];
            // TID: 1, PID: 0, LENGTH: 5, UnitID: 2, FC: 3, ByteCount: 2, Registers: [30]
            let expected_response_for_op_b = [
                0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x02, 0x03, 0x02, 0x00, 0x1e,
            ];

            serve_exchanges(
                server_stream,
                &[
                    (expected_request_from_op_a, &expected_response_for_op_a),
                    (expected_request_from_op_b, &expected_response_for_op_b),
                ],
            );
        });

        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("client should connect");
        let operation_a = ModbusOperation::new(
            0x01,
            crate::FunctionCode::ReadHoldingRegisters,
            0x0000,
            0x0002,
        )
        .expect("valid operation A");
        let op_a_response = session
            .execute(&operation_a)
            .expect("valid response should be a protocol outcome");
        assert_eq!(op_a_response.registers(), Some(&[10u16, 20][..]));
        let operation_b = ModbusOperation::new(
            0x02,
            crate::FunctionCode::ReadHoldingRegisters,
            0x000a,
            0x0001,
        )
        .expect("valid operation B");
        let op_b_response = session
            .execute(&operation_b)
            .expect("valid response should be a protocol outcome");
        assert_eq!(op_b_response.registers(), Some(&[30u16][..]));

        server_thread
            .join()
            .expect("server thread panicked or failed");
    }
}
