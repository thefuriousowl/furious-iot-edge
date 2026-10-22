use crate::{
    modbus_tcp::{
        FunctionCode::{ReadHoldingRegisters, ReadInputRegisters},
        ModbusError, ModbusException, ModbusOperation, ModbusSession,
    },
    scheduler::{ScheduledOp, Scheduler},
    shared_state::{RegisterSnapshot, SnapshotStore},
};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    thread,
    time::{Duration, Instant},
};

const GLOBAL_TEST_READ_TIMEOUT: Duration = Duration::from_millis(200);
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

fn create_tcp_test_client(server_addr: SocketAddr) -> ModbusSession {
    ModbusSession::connect(&server_addr.to_string(), GLOBAL_TEST_READ_TIMEOUT)
        .expect("should connect to test server")
}

fn retained(store: &SnapshotStore, operation_id: &str) -> Option<Vec<u16>> {
    store
        .get(operation_id)
        .map(|snapshot| snapshot.registers().to_vec())
}

fn read_request(tid: u16, uid: u8, fc: u8, start: u16, qty: u16) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes[0..2].copy_from_slice(&tid.to_be_bytes());
    bytes[4..6].copy_from_slice(&(6u16).to_be_bytes());
    bytes[6] = uid;
    bytes[7] = fc;
    bytes[8..10].copy_from_slice(&start.to_be_bytes());
    bytes[10..12].copy_from_slice(&qty.to_be_bytes());
    bytes
}

fn fc03_request(tid: u16, uid: u8, start: u16, qty: u16) -> [u8; 12] {
    read_request(tid, uid, 0x03, start, qty)
}
fn fc04_request(tid: u16, uid: u8, start: u16, qty: u16) -> [u8; 12] {
    read_request(tid, uid, 0x04, start, qty)
}

fn register_response(tid: u16, uid: u8, fc: u8, registers: &[u16]) -> Vec<u8> {
    let byte_count = (registers.len() * 2) as u8;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&tid.to_be_bytes());
    bytes.extend_from_slice(&(0u16).to_be_bytes()); // PID
    bytes.extend_from_slice(&(3 + (byte_count as u16)).to_be_bytes()); // UID + FC + ByteCount + data
    bytes.push(uid);
    bytes.push(fc);
    bytes.push(byte_count);
    for r in registers {
        bytes.extend_from_slice(&r.to_be_bytes());
    }
    bytes
}

fn exception_response(tid: u16, uid: u8, fc: u8, code: u8) -> Vec<u8> {
    // MBAP Length = 3 (UID + FC | 0x80 + exception code)
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&tid.to_be_bytes());
    bytes.extend_from_slice(&(0u16).to_be_bytes());
    bytes.extend_from_slice(&(3u16).to_be_bytes());
    bytes.push(uid);
    bytes.push(0x80 | fc);
    bytes.push(code);
    bytes
}

#[test]
fn invalid_response_on_usable_connection_keeps_polling() {
    let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
        let first_req = fc03_request(0, 1, 0, 2);
        let first_res_valid = register_response(0, 1, 3, &[10, 20]);
        let second_req = fc03_request(1, 1, 0, 2);
        let mut second_res_invalid_byte_count = register_response(1, 1, 3, &[30, 40]);
        second_res_invalid_byte_count[8] = 0x03;
        let third_req = fc03_request(2, 1, 0, 2);
        let third_res_valid = register_response(2, 1, 3, &[50, 60]);
        serve_exchanges(
            server_stream,
            &[
                (first_req, &first_res_valid),
                (second_req, &second_res_invalid_byte_count),
                (third_req, &third_res_valid),
            ],
        )
    });
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
    let mut session = create_tcp_test_client(server_addr);
    let scheduled_operation = ScheduledOp::new("op-1", operation, Duration::from_millis(100))
        .expect("valid scheduled operation");
    let mut scheduler = Scheduler::new();
    scheduler.add_operation(scheduled_operation);
    let store = SnapshotStore::new();
    let result = scheduler.run_all(&mut session, &store);
    assert!(matches!(result, Err(ModbusError::Io(_))));
    assert_eq!(retained(&store, "op-1"), Some(vec![50, 60]));
    server_thread.join().expect("server panicked or failed");
}

#[test]
fn silent_server_returns_read_timeout_error() {
    let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
    let server_addr = server
        .local_addr()
        .expect("should get test server local address");
    let server_thread = thread::spawn(move || {
        let (mut server_stream, _) = server.accept().expect("should accept client connection");
        let mut request = [0u8; 12];
        server_stream
            .read_exact(&mut request)
            .expect("should read request");

        thread::sleep(GLOBAL_TEST_READ_TIMEOUT * 2);
    });

    let mut session = create_tcp_test_client(server_addr);
    let response = session
        .execute(&ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid request"));

    assert!(
        matches!(response, Err(ModbusError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock || error.kind() == std::io::ErrorKind::TimedOut)
    );
    server_thread.join().expect("server panicked or failed");
}
#[test]
fn persist_success_operation_snapshot_after_exception_occurred() {
    let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
        let first_fc03_req_valid = fc03_request(0, 1, 0, 2);
        let first_fc03_res_valid = register_response(0, 1, 3, &[10u16, 20]);
        let second_fc03_req_valid = fc03_request(1, 1, 0, 2);
        let second_fc03_exception_valid = exception_response(1, 1, 3, 2);

        serve_exchanges(
            server_stream,
            &[
                (first_fc03_req_valid, &first_fc03_res_valid),
                (second_fc03_req_valid, &second_fc03_exception_valid),
            ],
        );
    });

    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
    let mut session = create_tcp_test_client(server_addr);
    let scheduled_operation = ScheduledOp::new("op-1", operation, Duration::from_millis(100))
        .expect("valid scheduled operation");
    let mut scheduler = Scheduler::new();
    scheduler.add_operation(scheduled_operation);
    let store = SnapshotStore::new();
    let result = scheduler.run_all(&mut session, &store);
    assert!(matches!(result, Err(ModbusError::Io(_))));
    assert_eq!(retained(&store, "op-1"), Some(vec![10u16, 20]));
    server_thread.join().expect("server panicked or failed");
}
#[test]
fn reject_incomplete_mbap_header() {
    let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
        let mut request = [0u8; 12];
        server_stream
            .read_exact(&mut request)
            .expect("should read request");
        // TID: 0, PID: 0, LENGTH: 7, missing unit ID
        let incomplete_response = [0x00, 0x00, 0x00, 0x00, 0x00, 0x07];
        server_stream
            .write_all(&incomplete_response)
            .expect("should write partial response");
        drop(server_stream);
    });
    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 3).expect("valid fc03 operation");
    let response = session.execute(&operation);
    assert!(
        matches!(response, Err(ModbusError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    server_thread
        .join()
        .expect("server thread panicked or failed");
}
#[test]
fn reject_invalid_mbap_length() {
    let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
    let server_addr = server
        .local_addr()
        .expect("should get local test server address");

    let server_thread = thread::spawn(move || {
        let invalid_mbap_lengths: [u16; 4] = [0, 1, 255, 65535];
        for item in invalid_mbap_lengths {
            let (mut server_stream, _) = server.accept().expect("should accept client connection");
            let mut request_buf = [0u8; 12];
            server_stream
                .read_exact(&mut request_buf)
                .expect("should read request");
            let fc03_req_valid = fc03_request(0, 1, 0, 2);
            assert_eq!(fc03_req_valid, request_buf);
            // TID: 0, PID: 0, LENGTH: invalid_mbap_lengths[i], UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
            let mut invalid_length_fc03_res = [0u8; 13];
            invalid_length_fc03_res[0..2].copy_from_slice(&(0u16).to_be_bytes());
            invalid_length_fc03_res[2..4].copy_from_slice(&(0u16).to_be_bytes());
            invalid_length_fc03_res[4..6].copy_from_slice(&item.to_be_bytes());
            invalid_length_fc03_res[6] = 0x01;
            invalid_length_fc03_res[7] = 0x03;
            invalid_length_fc03_res[8] = 0x04;
            invalid_length_fc03_res[9..11].copy_from_slice(&(10u16).to_be_bytes());
            invalid_length_fc03_res[11..13].copy_from_slice(&(20u16).to_be_bytes());
            server_stream
                .write_all(&invalid_length_fc03_res)
                .expect("should write response");
        }
    });

    for i in 0..4 {
        let mut session = create_tcp_test_client(server_addr);
        let operation =
            ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
        let response = session.execute(&operation);
        match i {
            0 => assert!(matches!(
                response,
                Err(ModbusError::InvalidMbapLength {
                    min: 2,
                    max: 254,
                    found: 0,
                })
            )),
            1 => assert!(matches!(
                response,
                Err(ModbusError::InvalidMbapLength {
                    min: 2,
                    max: 254,
                    found: 1,
                })
            )),
            2 => assert!(matches!(
                response,
                Err(ModbusError::InvalidMbapLength {
                    min: 2,
                    max: 254,
                    found: 255,
                })
            )),
            3 => assert!(matches!(
                response,
                Err(ModbusError::InvalidMbapLength {
                    min: 2,
                    max: 254,
                    found: 65535,
                })
            )),
            _ => panic!("panic if i not in range 0..4"),
        }
    }

    server_thread.join().expect("server thread failed");
}
#[test]
fn short_pdu_then_valid_fc03_on_same_connection() {
    let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
        // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let first_fc03_req_valid = fc03_request(0, 1, 0, 2);
        let truncated_exception_byte_pdu = [0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x83];
        let second_fc03_req_valid = fc03_request(1, 1, 0, 2);
        // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
        let second_fc03_res_valid = register_response(1, 1, 3, &[10u16, 20]);
        serve_exchanges(
            server_stream,
            &[
                (first_fc03_req_valid, &truncated_exception_byte_pdu),
                (second_fc03_req_valid, &second_fc03_res_valid),
            ],
        );
    });
    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid FC03 operation");
    let response = session.execute(&operation);
    assert!(matches!(
        response,
        Err(ModbusError::PduTruncated { got_len: 1 })
    ));
    let response = session
        .execute(&operation)
        .expect("should get valid response");
    assert_eq!(response.registers(), Some(&[10u16, 20][..]));
    server_thread.join().expect("server thread panic or failed");
}
#[test]
fn success_transaction_after_reject_malformed_exception() {
    let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
        // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let first_fc03_req_valid = fc03_request(0, 1, 0, 2);
        let mut first_valid_exception_res = exception_response(0, 1, 3, 2);
        first_valid_exception_res.extend_from_slice(&[0x00]);
        first_valid_exception_res[4..6].copy_from_slice(&(4u16).to_be_bytes());
        let second_fc03_req_valid = fc03_request(1, 1, 0, 2);
        // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
        let second_fc03_res_valid = register_response(1, 1, 3, &[10u16, 20]);
        serve_exchanges(
            server_stream,
            &[
                (first_fc03_req_valid, &first_valid_exception_res),
                (second_fc03_req_valid, &second_fc03_res_valid),
            ],
        );
    });

    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid FC03 operation");
    let response = session.execute(&operation);
    assert!(matches!(
        response,
        Err(ModbusError::InvalidExceptionPduLen {
            expected: 2,
            found: 3,
        })
    ));
    let response = session
        .execute(&operation)
        .expect("should get valid response");
    assert_eq!(response.registers(), Some(&[10u16, 20][..]));
    server_thread.join().expect("server thread panic or failed");
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
    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 3).expect("valid fc03 operation");
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
        let fc03_req_valid = fc03_request(0, 1, 0, 2);
        // TID: 0, PID: 0, LENGTH: 7, UnitID: 2, FC: 3, ByteCount: 4, Data: [10, 20]
        let fc03_res_uid_mismatch = register_response(0, 2, 3, &[10u16, 20]);
        serve_exchanges(server_stream, &[(fc03_req_valid, &fc03_res_uid_mismatch)]);
    });
    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid FC03 operation");
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
        let fc03_req_valid = fc03_request(0, 1, 0, 2);
        // TID: 0, PID: 1, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
        let mut fc03_res_mismatch_pid = register_response(0, 1, 3, &[10u16, 20]);
        fc03_res_mismatch_pid[2..4].copy_from_slice(&(1u16).to_be_bytes());

        serve_exchanges(server_stream, &[(fc03_req_valid, &fc03_res_mismatch_pid)]);
    });
    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid FC03 operation");
    let response = session.execute(&operation);
    assert!(matches!(
        response,
        Err(ModbusError::InvalidProtocolId {
            expected: 0x0000,
            found: 0x0001,
        })
    ));
    server_thread.join().expect("server thread panic or failed");
}
#[test]
fn reject_mismatch_transaction_id() {
    let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
        // TID: 0, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let fc03_req_valid = fc03_request(0, 1, 0, 2);
        // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Data: [10, 20]
        let fc03_res_mismatch_tid = register_response(1, 1, 3, &[10u16, 20]);
        serve_exchanges(server_stream, &[(fc03_req_valid, &fc03_res_mismatch_tid)]);
    });
    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid FC03 operation");
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
        let first_fc03_req_valid = fc03_request(0, 1, 0, 2);
        // Requested quantity: 2 registers (4 data bytes); response Byte Count: 2.
        // TID: 0, PID: 0, LENGTH: 5, UnitID: 1, FC: 3, ByteCount: 2, Registers: [10]
        let first_fc03_res_quantity_mismatch = register_response(0, 1, 3, &[10u16]);
        // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let second_fc03_req_valid = fc03_request(1, 1, 0, 2);
        // Response Byte Count: 4; actual response data: 2 bytes (1 register).
        // TID: 1, PID: 0, LENGTH: 5, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10]
        let mut second_fc03_res_byte_count_mismatch = register_response(1, 1, 3, &[10u16]);
        second_fc03_res_byte_count_mismatch[8] = 0x04;
        serve_exchanges(
            server_stream,
            &[
                (first_fc03_req_valid, &first_fc03_res_quantity_mismatch),
                (second_fc03_req_valid, &second_fc03_res_byte_count_mismatch),
            ],
        );
    });
    let mut session = create_tcp_test_client(server_addr);
    let operation_a =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
    let response = session.execute(&operation_a);
    assert!(matches!(
        response,
        Err(ModbusError::ByteCountMismatched {
            expected: 4,
            found: 2,
        })
    ));

    let operation_b =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
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
        let first_fc04_req_valid = fc04_request(0, 1, 0, 2);
        // request fc04 but response fc03
        // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
        let first_res_mismatch_fc = register_response(0, 1, 3, &[10u16, 20]);
        // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let second_fc03_req_valid = fc03_request(1, 1, 0, 2);
        // request fc03 but response fc04
        // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 4, ByteCount: 4, Registers: [10, 20]
        let second_res_mismatch_fc = register_response(1, 1, 4, &[10u16, 20]);
        // TID: 2, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let third_fc03_req_valid = fc03_request(2, 1, 0, 2);
        // request fc03 but response fc04 exception
        // TID: 2, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x04), ExceptionCode: 2
        let third_res_mismatch_fc_exception = exception_response(2, 1, 4, 2);
        // TID: 3, PID: 0, LENGTH: 6, UnitID: 1, FC: 4, Start: 0, Quantity: 2
        let fourth_fc04_req_valid = fc04_request(3, 1, 0, 2);
        // request fc04 but response fc03 exception
        // TID: 3, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x03), ExceptionCode: 2
        let fourth_res_mismatch_fc_exception = exception_response(3, 1, 3, 2);
        serve_exchanges(
            server_stream,
            &[
                (first_fc04_req_valid, &first_res_mismatch_fc),
                (second_fc03_req_valid, &second_res_mismatch_fc),
                (third_fc03_req_valid, &third_res_mismatch_fc_exception),
                (fourth_fc04_req_valid, &fourth_res_mismatch_fc_exception),
            ],
        );
    });

    let operation_a =
        ModbusOperation::new(1, ReadInputRegisters, 0, 2).expect("should be valid fc04 operation");
    let mut session = create_tcp_test_client(server_addr);
    let response = session.execute(&operation_a);
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x04,
            response: 0x03,
        })
    ));

    let operation_b = ModbusOperation::new(1, ReadHoldingRegisters, 0, 2)
        .expect("should be valid fc03 operation");
    let response = session.execute(&operation_b);
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x03,
            response: 0x04,
        })
    ));

    let operation_c = ModbusOperation::new(1, ReadHoldingRegisters, 0, 2)
        .expect("should be valid fc03 operation");
    let response = session.execute(&operation_c);
    assert!(matches!(
        response,
        Err(ModbusError::FunctionCodeMismatch {
            request: 0x03,
            response: 0x84,
        })
    ));

    let operation_d =
        ModbusOperation::new(1, ReadInputRegisters, 0, 2).expect("should be valid fc04 operation");
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
        let first_fc03_req_valid = fc04_request(0, 1, 16, 2);
        // TID: 0, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x04), ExceptionCode: 2
        let first_fc04_exception_valid = exception_response(0, 1, 4, 2);
        // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 4, Start: 16, Quantity: 2
        let second_fc04_req_valid = fc04_request(1, 1, 16, 2);
        // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 4, ByteCount: 4, Registers: [10, 20]
        let second_fc04_res_valid = register_response(1, 1, 4, &[10u16, 20]);
        serve_exchanges(
            server_stream,
            &[
                (first_fc03_req_valid, &first_fc04_exception_valid),
                (second_fc04_req_valid, &second_fc04_res_valid),
            ],
        );
    });

    let fc04_operation =
        ModbusOperation::new(1, ReadInputRegisters, 16, 2).expect("should be valid operation");
    let mut session = create_tcp_test_client(server_addr);
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
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid operation");
    let fc04_operation =
        ModbusOperation::new(2, ReadInputRegisters, 0, 2).expect("valid operation");

    let mut session = create_tcp_test_client(server_addr);
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
        let fc03_req_valid = fc03_request(0, 1, 0, 2);
        // Modbus TCP Response ADU (2 registers = 4 bytes response data)
        // Header (7B) + FC (1B) + Byte Count (1B) + Register Data (4B) = 13 bytes
        // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
        let fc03_res_valid = register_response(0, 1, 3, &[10u16, 20]);
        serve_exchanges(server_stream, &[(fc03_req_valid, &fc03_res_valid)]);
    });

    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(0x01, ReadHoldingRegisters, 0x0000, 0x0002).expect("valid operation");
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
        let first_fc03_req_valid = fc03_request(0, 1, 0, 2);
        // TID: 0, PID: 0, LENGTH: 3, UnitID: 1, FC: Exception (0x80 | 0x03), ExceptionCode: 2
        let first_exception_res_valid = exception_response(0, 1, 3, 2);
        // TID: 1, PID: 0, LENGTH: 6, UnitID: 1, FC: 3, Start: 0, Quantity: 2
        let second_fc03_req_valid = fc03_request(1, 1, 0, 2);
        // TID: 1, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
        let second_fc03_res_valid = register_response(1, 1, 3, &[10u16, 20]);
        serve_exchanges(
            server_stream,
            &[
                (first_fc03_req_valid, &first_exception_res_valid),
                (second_fc03_req_valid, &second_fc03_res_valid),
            ],
        );
    });

    let mut session = create_tcp_test_client(server_addr);
    let operation =
        ModbusOperation::new(0x01, ReadHoldingRegisters, 0x0000, 0x0002).expect("valid operation");
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
        let op_a_fc03_req_valid = fc03_request(0, 1, 0, 2);
        // TID: 0, PID: 0, LENGTH: 7, UnitID: 1, FC: 3, ByteCount: 4, Registers: [10, 20]
        let op_a_fc03_res_valid = register_response(0, 1, 3, &[10u16, 20]);
        // TID: 1, PID: 0, LENGTH: 6, UnitID: 2, FC: 3, Start: 10, Quantity: 1
        let op_b_fc03_req_valid = fc03_request(1, 2, 10, 1);
        // TID: 1, PID: 0, LENGTH: 5, UnitID: 2, FC: 3, ByteCount: 2, Registers: [30]
        let op_b_fc03_res_valid = register_response(1, 2, 3, &[30u16]);
        serve_exchanges(
            server_stream,
            &[
                (op_a_fc03_req_valid, &op_a_fc03_res_valid),
                (op_b_fc03_req_valid, &op_b_fc03_res_valid),
            ],
        );
    });

    let mut session = create_tcp_test_client(server_addr);
    let operation_a = ModbusOperation::new(0x01, ReadHoldingRegisters, 0x0000, 0x0002)
        .expect("valid operation A");
    let op_a_response = session
        .execute(&operation_a)
        .expect("valid response should be a protocol outcome");
    assert_eq!(op_a_response.registers(), Some(&[10u16, 20][..]));
    let operation_b = ModbusOperation::new(0x02, ReadHoldingRegisters, 0x000a, 0x0001)
        .expect("valid operation B");
    let op_b_response = session
        .execute(&operation_b)
        .expect("valid response should be a protocol outcome");
    assert_eq!(op_b_response.registers(), Some(&[30u16][..]));

    server_thread
        .join()
        .expect("server thread panicked or failed");
}
#[test]
fn transaction_id_mismatch_keeps_seeded_snapshot() {
    let (server_addr, server_thread) = create_tcp_test_server(|stream| {
        let request = fc03_request(0, 1, 0, 2);
        let response = register_response(5, 1, 3, &[30, 40]);

        serve_exchanges(stream, &[(request, &response)]);
    });

    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
    let mut session = create_tcp_test_client(server_addr);
    let scheduled_operation = ScheduledOp::new("op-1", operation, Duration::from_millis(100))
        .expect("valid scheduled operation");

    let mut scheduler = Scheduler::new();

    let mut data = [0u16; 125];
    data[0] = 10;
    data[1] = 20;
    let seeded_at = Instant::now();
    let store = SnapshotStore::new();
    store.put("op-1", RegisterSnapshot::from(data, 2, seeded_at));
    scheduler.add_operation(scheduled_operation);
    let result = scheduler.run_all(&mut session, &store);
    assert!(matches!(
        result,
        Err(ModbusError::TransactionIdMismatch {
            expected: 0,
            found: 5,
        })
    ));

    let snapshot = store.get("op-1").expect("seeded snapshot should remain");
    assert_eq!(snapshot.registers(), &[10u16, 20]);
    assert_eq!(snapshot.acquired_at(), seeded_at);

    server_thread.join().expect("server panicked or failed");
}
#[test]
fn last_success_stays_none_after_first_poll_exception() {
    let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
    let server_addr = server
        .local_addr()
        .expect("should get local server address");
    let server_thread = thread::spawn(move || {
        let (mut server_stream, _) = server.accept().expect("should accept client connection");
        let mut request = [0u8; 12];
        server_stream
            .read_exact(&mut request)
            .expect("should read request");

        let exception_response_adu = [0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01, 0x83, 0x02];
        server_stream
            .write_all(&exception_response_adu)
            .expect("should write exception response");
    });

    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
    let mut session = create_tcp_test_client(server_addr);
    let scheduled_operation = ScheduledOp::new("op-1", operation, Duration::from_millis(100))
        .expect("valid scheduled operation");

    let mut scheduler = Scheduler::new();
    scheduler.add_operation(scheduled_operation);
    let store = SnapshotStore::new();
    let result = scheduler.run_all(&mut session, &store);
    assert!(matches!(result, Err(ModbusError::Io(_))));
    assert_eq!(retained(&store, "op-1"), None);

    server_thread.join().expect("server panicked or failed");
}

#[test]
fn last_success_stays_none_after_first_poll_transaction_id_mismatch() {
    let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
    let server_addr = server
        .local_addr()
        .expect("should get local server address");
    let server_thread = thread::spawn(move || {
        let (mut server_stream, _) = server.accept().expect("should accept client connection");
        let mut request = [0u8; 12];
        server_stream
            .read_exact(&mut request)
            .expect("should read request");

        // wrong TID (5), carries [30, 40]
        let invalid_response_adu = [
            0x00, 0x05, 0x00, 0x00, 0x00, 0x07, 0x01, 0x03, 0x04, 0x00, 0x1e, 0x00, 0x28,
        ];
        server_stream
            .write_all(&invalid_response_adu)
            .expect("should write invalid response");
    });

    let operation =
        ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid fc03 request");
    let mut session = create_tcp_test_client(server_addr);
    let scheduled_operation = ScheduledOp::new("op-1", operation, Duration::from_millis(100))
        .expect("valid scheduled operation");

    let mut scheduler = Scheduler::new();
    scheduler.add_operation(scheduled_operation);
    let store = SnapshotStore::new();
    let result = scheduler.run_all(&mut session, &store);
    assert!(matches!(
        result,
        Err(ModbusError::TransactionIdMismatch { .. })
    ));
    assert_eq!(retained(&store, "op-1"), None);
    server_thread.join().expect("server panicked or failed");
}

#[test]
fn update_collect_operation() {
    let (server_addr, server_thread) = create_tcp_test_server(move |server_stream| {
        let op_a_fc03_req_valid = fc03_request(0, 1, 0, 2);
        let op_a_fc03_res_valid = register_response(0, 1, 3, &[10u16, 20]);
        let op_b_fc04_req_valid = fc04_request(1, 2, 0, 1);
        let op_b_fc04_res_valid = register_response(1, 2, 4, &[30u16]);
        serve_exchanges(
            server_stream,
            &[
                (op_a_fc03_req_valid, &op_a_fc03_res_valid),
                (op_b_fc04_req_valid, &op_b_fc04_res_valid),
            ],
        );
    });
    let expected_op_1_id = "op-1";
    let expected_op_2_id = "op-2";
    let operation_a = ModbusOperation::new(1, ReadHoldingRegisters, 0, 2).expect("valid operation");
    let scheduled_op_a =
        ScheduledOp::new(expected_op_1_id, operation_a, Duration::from_millis(1000))
            .expect("valid scheduled op");
    let operation_b = ModbusOperation::new(2, ReadInputRegisters, 0, 1).expect("valid operation");
    let scheduled_op_b =
        ScheduledOp::new(expected_op_2_id, operation_b, Duration::from_millis(1000))
            .expect("valid scheduled op");
    let mut scheduler = Scheduler::new();
    scheduler.add_operation(scheduled_op_a);
    scheduler.add_operation(scheduled_op_b);
    let mut session = create_tcp_test_client(server_addr);
    let store = SnapshotStore::new();
    let result = scheduler.run_all(&mut session, &store);
    assert!(matches!(result, Err(ModbusError::Io(..))));
    assert_eq!(retained(&store, expected_op_1_id), Some(vec![10u16, 20]));
    assert_eq!(scheduler.ops()[0].operation_id(), expected_op_1_id);
    assert_eq!(scheduler.ops()[1].operation_id(), expected_op_2_id);
    assert_eq!(retained(&store, expected_op_2_id), Some(vec![30u16]));
    server_thread
        .join()
        .expect("server thread panicked or failed");
}
