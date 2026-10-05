use crate::{MB_TCP_PID, ModbusError, ModbusException, ModbusOperation, ModbusSession};
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
#[test]
fn reject_truncated_response_from_unexpected_connection_close() {
    let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
        let mut request = [0u8; 12];
        server_stream
            .read_exact(&mut request)
            .expect("should read request");

        // TID: 0, PID: 0, LENGTH: 7, UnitID: 1
        // declared 6 bytes PDU but actual sent 4 bytes
        let incomplete_response = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x0a,
        ];

        server_stream
            .write_all(&incomplete_response)
            .expect("should write partial response");

        drop(server_stream);
    });

    let mut session =
        ModbusSession::connect(&server_addr.to_string()).expect("should connect to test server");
    let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 3)
        .expect("valid fc03 operation");
    let response = session.execute(&operation);
    assert!(
        matches!(response, Err(ModbusError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    server_thread
        .join()
        .expect("server thread panicked or failed");
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

    let mut session =
        ModbusSession::connect(&server_addr.to_string()).expect("should connect to test server");
    let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("valid FC03 operation");
    let response = session.execute(&operation);
    assert!(matches!(
        response,
        Err(ModbusError::UnitIdMismatch {
            expected: 1,
            found: 2,
        })
    ));

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

    let mut session =
        ModbusSession::connect(&server_addr.to_string()).expect("should connect to test server");
    let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("valid FC03 operation");
    let response = session.execute(&operation);

    assert!(matches!(
        response,
        Err(ModbusError::InvalidProtocolId {
            expected: MB_TCP_PID,
            found: 0x0001,
        })
    ));
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

    let mut session =
        ModbusSession::connect(&server_addr.to_string()).expect("should connect to test server");
    let operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("valid FC03 operation");
    let response = session.execute(&operation);
    assert!(matches!(
        response,
        Err(ModbusError::TransactionIdMismatch {
            expected: 0x0000,
            found: 0x0001,
        })
    ));
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
    let mut session =
        ModbusSession::connect(&server_addr.to_string()).expect("should connect to test server");
    let operation_a = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("valid fc03 request");
    let response = session.execute(&operation_a);
    assert!(matches!(
        response,
        Err(ModbusError::ByteCountMismatched {
            expected: 4,
            found: 2,
        })
    ));

    let operation_b = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("valid fc03 request");
    let response = session.execute(&operation_b);
    assert!(matches!(
        response,
        Err(ModbusError::ActualDataLenMismatchByteCount {
            byte_count: 4,
            data_len: 2,
        })
    ));
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
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x04,
            response: 0x03,
        })
    ));

    let operation_b = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("should be valid fc03 operation");
    let response = session.execute(&operation_b);
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x03,
            response: 0x04,
        })
    ));

    let operation_c = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
        .expect("should be valid fc03 operation");
    let response = session.execute(&operation_c);
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x03,
            response: 0x84,
        })
    ));

    let operation_d = ModbusOperation::new(1, crate::FunctionCode::ReadInputRegisters, 0, 2)
        .expect("should be valid fc04 operation");
    let response = session.execute(&operation_d);
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x04,
            response: 0x83,
        })
    ));

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

    let fc04_operation = ModbusOperation::new(1, crate::FunctionCode::ReadInputRegisters, 16, 2)
        .expect("should be valid response");
    let mut session =
        ModbusSession::connect(&server_addr.to_string()).expect("should connect to test server");
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

    let fc03_operation = ModbusOperation::new(1, crate::FunctionCode::ReadHoldingRegisters, 0, 2)
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
