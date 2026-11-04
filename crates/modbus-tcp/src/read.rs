use std::{
    io::{Read, Write},
    net::TcpStream,
};

use crate::{
    codec::{assemble_read_request_pdu, assemble_request_adu, assemble_request_mbap},
    definition::ReadRequestDefinition,
    error::{
        ModbusTcpError::{self},
        RequestError, ResponseError, SyncError,
    },
    helper::ensure,
};

pub const MBAP_HEADER_LEN: usize = 7;
pub const MODBUS_TCP_PID: u16 = 0;
#[derive(Debug)]
enum ResultType {
    Coils,
    Register,
}

#[derive(Debug)]
pub struct ReadResult {
    bytes_data: [u8; 250],
    count: u16,
    result_type: ResultType,
}
impl ReadResult {
    fn new(bytes_data: [u8; 250], count: u16, result_type: ResultType) -> Self {
        Self {
            bytes_data,
            count,
            result_type,
        }
    }

    pub fn bytes_data(&self) -> &[u8] {
        match self.result_type {
            ResultType::Coils => &self.bytes_data[..self.count.div_ceil(8) as usize],
            ResultType::Register => &self.bytes_data[..(self.count as usize) * 2],
        }
    }

    pub fn count(&self) -> u16 {
        self.count
    }
}

pub fn try_read_once(
    stream: &mut TcpStream,
    definition: &ReadRequestDefinition,
    request_tid: u16,
) -> Result<ReadResult, ModbusTcpError> {
    let max_quantity = match definition.function_code {
        0x01 | 0x02 => 2000,
        0x03 | 0x04 => 125,
        fc => {
            return Err(RequestError::UnexpectedFunctionCode { found: fc }.into());
        }
    };

    ensure(
        (1..=max_quantity).contains(&definition.quantity),
        RequestError::InvalidQuantity {
            min: 1,
            max: max_quantity,
            found: definition.quantity,
        },
    )?;

    let check_range_add = u32::from(definition.starting_address) + u32::from(definition.quantity);

    if check_range_add > 65536 {
        return Err(RequestError::AddressOutOfRange {
            min: 1,
            max: 65535,
            found: check_range_add,
        }
        .into());
    }

    let request_pdu = assemble_read_request_pdu(
        definition.function_code,
        definition.starting_address,
        definition.quantity,
    );

    let request_mbap = assemble_request_mbap(request_tid, definition.unit_id, 6);
    let request_adu = assemble_request_adu(request_mbap, request_pdu);
    stream.write_all(&request_adu)?;

    // Modbus Response
    let mut response_mbap = [0u8; MBAP_HEADER_LEN];
    stream.read_exact(&mut response_mbap)?;

    let response_length = u16::from_be_bytes([response_mbap[4], response_mbap[5]]);
    ensure(
        (3..=254).contains(&response_length),
        SyncError::InvalidMbapLength {
            min: 3,
            max: 254,
            found: response_length,
        },
    )?;

    let response_pdu_len = usize::from(response_length - 1);
    let mut response_pdu = [0u8; 253];
    stream.read_exact(&mut response_pdu[..response_pdu_len])?;

    // Read and validate response transaction ID
    let response_tid = u16::from_be_bytes([response_mbap[0], response_mbap[1]]);
    ensure(
        response_tid == request_tid,
        SyncError::TransactionIdMismatch {
            expected: request_tid,
            found: response_tid,
        },
    )?;

    // Read and validate response protocol ID
    let response_pid = u16::from_be_bytes([response_mbap[2], response_mbap[3]]);
    ensure(
        response_pid == MODBUS_TCP_PID,
        ResponseError::InvalidProtocolId {
            expected: MODBUS_TCP_PID,
            found: response_pid,
        },
    )?;

    // Read and validate unit ID
    let response_uid = response_mbap[6];
    ensure(
        response_uid == definition.unit_id,
        ResponseError::UnitIdMismatch {
            expected: definition.unit_id,
            found: response_uid,
        },
    )?;

    // Read and validate function code
    let response_fc = response_pdu[0];
    let exception_fc = definition.function_code | 0x80;
    if response_fc == exception_fc {
        ensure(
            response_pdu_len == 2,
            ResponseError::InvalidExceptionPduLen {
                expected: 2,
                found: response_pdu_len,
            },
        )?;

        return Err(ModbusTcpError::Exception(response_pdu[1].into()));
    }

    ensure(
        response_fc == definition.function_code,
        ResponseError::FunctionCodeMismatch {
            request: definition.function_code,
            response: response_fc,
        },
    )?;

    let response_byte_count = response_pdu[1];

    // validate response function code
    let expected_byte_count = match definition.function_code {
        0x01 | 0x02 => usize::from(definition.quantity.div_ceil(8)),
        0x03 | 0x04 => usize::from(definition.quantity) * 2,
        _ => unreachable!(),
    };

    let data_len = response_pdu_len - 2;

    ensure(
        usize::from(response_byte_count) == expected_byte_count,
        ResponseError::ByteCountMismatched {
            expected: expected_byte_count as u16,
            found: u16::from(response_byte_count),
        },
    )?;

    ensure(
        data_len == usize::from(response_byte_count),
        ResponseError::ActualDataLenMismatchByteCount {
            byte_count: usize::from(response_byte_count),
            data_len,
        },
    )?;

    let response_data = &response_pdu[2..response_pdu_len];

    let mut bytes = [0u8; 250];
    bytes[..data_len].copy_from_slice(response_data);
    Ok(ReadResult::new(
        bytes,
        definition.quantity,
        match response_fc {
            0x01 | 0x02 => ResultType::Coils,
            0x03 | 0x04 => ResultType::Register,
            _ => unreachable!(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
    };

    use crate::{
        ReadRequestDefinition,
        error::{
            ModbusException,
            ModbusTcpError::{Exception, Io, Request, Response, Sync},
            RequestError, ResponseError, SyncError,
        },
        try_read_once,
    };

    fn read_request_builder(tid: u16, uid: u8, fc: u8, start: u16, qty: u16) -> [u8; 12] {
        let mut request = [0u8; 12];
        request[0..2].copy_from_slice(&tid.to_be_bytes());
        request[2..4].copy_from_slice(&(0u16).to_be_bytes());
        request[4..6].copy_from_slice(&(6u16).to_be_bytes()); // request mbap length
        request[6] = uid;
        request[7] = fc;
        request[8..10].copy_from_slice(&start.to_be_bytes());
        request[10..12].copy_from_slice(&qty.to_be_bytes());
        request
    }

    fn read_response_builder(tid: u16, uid: u8, fc: u8, response_data: &[u8]) -> Vec<u8> {
        let byte_count = response_data.len() as u8;
        let mut response = Vec::new();
        response.extend_from_slice(&tid.to_be_bytes());
        response.extend_from_slice(&(0u16).to_be_bytes());
        response.extend_from_slice(&(3 + (byte_count as u16)).to_be_bytes()); // response mbap length
        response.push(uid);
        response.push(fc);
        response.push(byte_count);
        response.extend_from_slice(response_data);
        response
    }

    fn exception_response_builder(tid: u16, uid: u8, fc: u8, exception_code: u8) -> Vec<u8> {
        let mut response = Vec::new();
        response.extend_from_slice(&tid.to_be_bytes());
        response.extend_from_slice(&(0u16).to_be_bytes());
        response.extend_from_slice(&(3u16).to_be_bytes()); // exception mbap length always 3 if exception occurs
        response.push(uid);
        response.push(fc | 0x80);
        response.push(exception_code);

        response
    }

    fn create_tcp_test_server(
        f: impl FnOnce(std::net::TcpStream) + Send + 'static,
    ) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("should bind local test server");
        let addr = listener.local_addr().expect("should return local address");

        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("should accept client connection");
            f(stream)
        });
        (addr, server_thread)
    }

    // happy path test
    #[test]
    fn read_coils_returns_expected_bytes_and_count() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 1, 0, 10);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 1, &[0x55, 0x01]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x01,
            starting_address: 0,
            quantity: 10,
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 10);
        assert_eq!(result.bytes_data(), &[0x55, 0x01]);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn read_discrete_inputs_returns_expected_bytes_and_count() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 2, 0, 10);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 2, &[0x55, 0x01]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x02,
            starting_address: 0,
            quantity: 10,
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 10);
        assert_eq!(result.bytes_data(), &[0x55, 0x01]);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn read_holding_registers_returns_expected_bytes_and_count() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 2);
        assert_eq!(result.bytes_data(), &[0x12, 0x34, 0x56, 0x78]);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn read_input_registers_returns_expected_bytes_and_count() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 4, 0, 2);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 4, &[0x12, 0x34, 0x56, 0x78]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x04,
            starting_address: 0,
            quantity: 2,
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 2);
        assert_eq!(result.bytes_data(), &[0x12, 0x34, 0x56, 0x78]);

        server_thread.join().expect("server thread panicked");
    }

    // response validation
    #[test]
    fn rejects_mismatch_tid() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(8, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Sync(SyncError::TransactionIdMismatch { .. })
            ),
            "{expected_error:?}"
        );
        assert!(!expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_mismatch_pid() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2); // received pid 0
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            actual_response[2..4].copy_from_slice(&(1u16).to_be_bytes()); // << wrong word here: send invalid pid 1
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Response(ResponseError::InvalidProtocolId { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_mismatch_uid() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2); // received uid 1
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            actual_response[6] = 2; // << wrong byte here: send invalid uid 2
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Response(ResponseError::UnitIdMismatch { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_mismatch_fc() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2); // received fc 3
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            actual_response[7] = 4; // << wrong byte here: send invalid fc 4
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Response(ResponseError::FunctionCodeMismatch { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_mismatch_byte_count() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2); // received qty 2 byte_count should be 4
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            actual_response[8] = 5; // << wrong byte here: send invalid byte_count 5
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Response(ResponseError::ByteCountMismatched { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn returns_device_exception() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2); // received qty 2 byte_count should be 4
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = exception_response_builder(7, 1, 3, 2);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Exception(ModbusException::IllegalDataAddress)
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    // request validation
    // should rejects invalid request before send to server
    #[test]
    fn rejects_unexpected_fc() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x05,
            starting_address: 0,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::UnexpectedFunctionCode { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_zero_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        // register
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 0,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::InvalidQuantity { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        // coil
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x01,
            starting_address: 0,
            quantity: 0,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::InvalidQuantity { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_exceeded_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        // register
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 126,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::InvalidQuantity { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        // coil
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x01,
            starting_address: 0,
            quantity: 2001,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::InvalidQuantity { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_exceeded_read_address() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        // register
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 65535,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::AddressOutOfRange { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        // coil
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x01,
            starting_address: 65535,
            quantity: 2,
        };
        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Request(RequestError::AddressOutOfRange { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());

        server_thread.join().expect("server thread panicked");
    }

    // happy path boundary validation

    #[test]
    fn accepts_max_coil_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 1, 0, 2000);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 1, &[255u8; 250]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");

        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x01,
            starting_address: 0,
            quantity: 2000, // max coil qty allowed
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 2000);
        assert_eq!(result.bytes_data(), &[255u8; 250]);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn accepts_max_register_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 125);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 3, &[255u8; 250]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 125,
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 125);
        assert_eq!(result.bytes_data(), &[255u8; 250]);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn accept_max_coil_address() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 1, 65535, 1);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 1, &[255u8]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");

        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x01,
            starting_address: 65535,
            quantity: 1, // max coil qty allowed
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 1);
        assert_eq!(result.bytes_data(), &[255u8; 1]);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn accept_max_register_address() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 65535, 1);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34]);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");

        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 65535,
            quantity: 1, // max register qty allowed
        };

        let result =
            try_read_once(&mut client_stream, &definition, 7).expect("read should succeed");

        assert_eq!(result.count(), 1);
        assert_eq!(result.bytes_data(), &[0x12, 0x34]);

        server_thread.join().expect("server thread panicked");
    }

    // validate response frame structure
    #[test]
    fn rejects_invalid_response_mbap_length() {
        for bad_mbap_length in [2u16, 255] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let expected_request = read_request_builder(7, 1, 3, 0, 2);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let mut actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
                actual_response[4..6].copy_from_slice(&bad_mbap_length.to_be_bytes()); // wrong word here as mbap length = 2 and 255
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let definition = ReadRequestDefinition {
                unit_id: 1,
                function_code: 0x03,
                starting_address: 0,
                quantity: 2,
            };

            let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
            assert!(
                matches!(
                    expected_error,
                    Sync(SyncError::InvalidMbapLength { found, .. }) if found == bad_mbap_length
                ),
                "{expected_error:?}"
            );
            assert!(!expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_exception_response_pdu_len() {
        let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = exception_response_builder(7, 1, 3, 2);
            actual_response[4..6].copy_from_slice(&4u16.to_be_bytes());
            actual_response.extend_from_slice(&[0x12]); // add another 1 byte to make pdu len wrong
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };

        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Response(ResponseError::InvalidExceptionPduLen { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn rejects_data_len_mismatch_from_byte_count() {
        let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
            let expected_request = read_request_builder(7, 1, 3, 0, 2);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);
            actual_response[4..6].copy_from_slice(&9u16.to_be_bytes());
            actual_response.extend_from_slice(&[0x56, 0x78]);

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let definition = ReadRequestDefinition {
            unit_id: 1,
            function_code: 0x03,
            starting_address: 0,
            quantity: 2,
        };

        let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
        assert!(
            matches!(
                expected_error,
                Response(ResponseError::ActualDataLenMismatchByteCount { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        server_thread.join().expect("server thread panicked");
    }

    // io error
    #[test]
    fn rejects_connection_closed_in_flight() {
        for truncated_index in [5, 9] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let expected_request = read_request_builder(7, 1, 3, 0, 2);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let actual_response = read_response_builder(7, 1, 3, &[0x12, 0x34, 0x56, 0x78]);

                server_stream
                    .write_all(&actual_response[..truncated_index])
                    .expect("should write response");

                drop(server_stream);
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let definition = ReadRequestDefinition {
                unit_id: 1,
                function_code: 0x03,
                starting_address: 0,
                quantity: 2,
            };

            let expected_error = try_read_once(&mut client_stream, &definition, 7).unwrap_err();
            assert!(
                matches!(expected_error, Io(_)),
                "cut at {truncated_index}: {expected_error:?}"
            );
            assert!(!expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }
}
