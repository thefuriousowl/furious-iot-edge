use crate::{
    MultipleWriteRequestDefinition,
    codec::{assemble_request_adu, assemble_request_mbap, assemble_single_write_request_pdu},
    definition::{MultipleWriteValues, SingleWriteRequestDefinition, SingleWriteValue},
    error::{ModbusTcpError, RequestError, ResponseError, SyncError},
    helper::ensure,
    read::{MBAP_HEADER_LEN, MODBUS_TCP_PID},
};
use std::{
    io::{Read, Write},
    net::TcpStream,
};

#[derive(Debug)]
pub struct WriteAck {
    unit_id: u8,
    starting_address: u16,
    quantity: u16,
}
impl WriteAck {
    pub fn new(unit_id: u8, starting_address: u16, quantity: u16) -> Self {
        Self {
            unit_id,
            starting_address,
            quantity,
        }
    }
    pub fn unit_id(&self) -> u8 {
        self.unit_id
    }

    pub fn starting_address(&self) -> u16 {
        self.starting_address
    }

    pub fn quantity(&self) -> u16 {
        self.quantity
    }
}

pub fn try_write_single_once(
    stream: &mut TcpStream,
    definition: &SingleWriteRequestDefinition,
    request_tid: u16,
) -> Result<WriteAck, ModbusTcpError> {
    let (function_code, wire_value): (u8, u16) = match &definition.value {
        SingleWriteValue::Coil(on) => (0x05, if *on { 0xff00 } else { 0x0000 }),
        SingleWriteValue::Register(value) => (0x06, *value),
    };

    let request_pdu =
        assemble_single_write_request_pdu(function_code, definition.starting_address, wire_value);
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

    if response_fc == (function_code | 0x80) {
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
        response_fc == function_code,
        ResponseError::FunctionCodeMismatch {
            request: function_code,
            response: response_fc,
        },
    )?;

    ensure(
        response_pdu_len == 5,
        ResponseError::InvalidWriteResponsePduLen {
            expected: 5,
            found: response_pdu_len,
        },
    )?;

    if response_pdu[..5] != request_pdu {
        let mut found = [0u8; 5];
        found.copy_from_slice(&response_pdu[..5]);
        return Err(ResponseError::WriteResponseMismatch {
            expected: request_pdu, // already [u8; 5] from assemble_single_write_request_pdu
            found,
        }
        .into());
    }

    Ok(WriteAck::new(response_uid, definition.starting_address, 1))
}

pub fn try_write_multiple_once(
    stream: &mut TcpStream,
    definition: &MultipleWriteRequestDefinition,
    request_tid: u16,
) -> Result<WriteAck, ModbusTcpError> {
    let mut pdu = [0u8; 253];

    let (function_code, quantity, byte_count) = match &definition.values {
        MultipleWriteValues::Coils(values) => {
            // FC15: Pack bool to Bits
            let quantity = values.len();

            ensure(
                (1..=1968).contains(&quantity),
                RequestError::InvalidQuantity {
                    min: 1,
                    max: 1968,
                    found: quantity.try_into().unwrap_or(u16::MAX),
                },
            )?;

            for (i, &on) in values.iter().enumerate() {
                if on {
                    pdu[6 + i / 8] |= 1u8 << (i % 8);
                }
            }

            (0x0f_u8, quantity as u16, quantity.div_ceil(8))
        }

        MultipleWriteValues::Registers(values) => {
            // FC16: Encode u16 as Big Endian
            let quantity = values.len();

            ensure(
                (1..=123).contains(&quantity),
                RequestError::InvalidQuantity {
                    min: 1,
                    max: 123,
                    found: quantity.try_into().unwrap_or(u16::MAX),
                },
            )?;

            for (i, &value) in values.iter().enumerate() {
                let offset = 6 + i * 2;

                pdu[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
            }

            (0x10_u8, quantity as u16, quantity * 2)
        }
    };

    let end_address = u32::from(definition.starting_address) + u32::from(quantity);

    ensure(
        end_address <= 65536,
        RequestError::AddressOutOfRange {
            min: 0,
            max: 65535,
            found: end_address,
        },
    )?;

    pdu[0] = function_code;
    pdu[1..3].copy_from_slice(&definition.starting_address.to_be_bytes());
    pdu[3..5].copy_from_slice(&quantity.to_be_bytes());
    pdu[5] = byte_count as u8;

    let request_pdu = &pdu[..6 + byte_count];
    let request_mbap_length = 1 + request_pdu.len();
    let request_mbap =
        assemble_request_mbap(request_tid, definition.unit_id, request_mbap_length as u16);

    let request_adu_len = MBAP_HEADER_LEN + request_pdu.len();

    let mut request_adu = [0u8; 260];
    request_adu[..MBAP_HEADER_LEN].copy_from_slice(&request_mbap);
    request_adu[MBAP_HEADER_LEN..request_adu_len].copy_from_slice(request_pdu);
    stream.write_all(&request_adu[..request_adu_len])?;

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
        definition.unit_id == response_uid,
        ResponseError::UnitIdMismatch {
            expected: definition.unit_id,
            found: response_uid,
        },
    )?;

    // Read and validate function code
    let response_fc = response_pdu[0];
    if response_fc == (function_code | 0x80) {
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
        response_fc == function_code,
        ResponseError::FunctionCodeMismatch {
            request: function_code,
            response: response_fc,
        },
    )?;

    ensure(
        response_pdu_len == 5,
        ResponseError::InvalidWriteResponsePduLen {
            expected: 5,
            found: response_pdu_len,
        },
    )?;

    if response_pdu[..5] != request_pdu[..5] {
        let mut expected = [0u8; 5];
        expected.copy_from_slice(&request_pdu[..5]); // request_pdu here is a slice
        let mut found = [0u8; 5];
        found.copy_from_slice(&response_pdu[..5]);
        return Err(ResponseError::WriteResponseMismatch { expected, found }.into());
    }

    Ok(WriteAck::new(
        response_uid,
        definition.starting_address,
        quantity,
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
        MultipleWriteRequestDefinition, MultipleWriteValues, SingleWriteRequestDefinition,
        SingleWriteValue,
        error::{
            ModbusException,
            ModbusTcpError::{Exception, Io, Request, Response, Sync},
            RequestError, ResponseError, SyncError,
        },
        try_write_multiple_once, try_write_single_once,
    };

    fn single_write_request_builder(
        tid: u16,
        uid: u8,
        start: u16,
        value: SingleWriteValue,
    ) -> [u8; 12] {
        let (function_code, wire_value): (u8, u16) = match &value {
            SingleWriteValue::Coil(on) => (0x05, if *on { 0xff00 } else { 0x0000 }),
            SingleWriteValue::Register(value) => (0x06, *value),
        };

        let mut request = [0u8; 12];

        request[0..2].copy_from_slice(&tid.to_be_bytes());
        request[2..4].copy_from_slice(&(0u16).to_be_bytes());
        request[4..6].copy_from_slice(&(6u16).to_be_bytes());
        request[6] = uid;
        request[7] = function_code;
        request[8..10].copy_from_slice(&start.to_be_bytes());
        request[10..12].copy_from_slice(&wire_value.to_be_bytes());

        request
    }

    fn single_write_response_builder(
        tid: u16,
        uid: u8,
        fc: u8,
        start: u16,
        value: u16,
    ) -> [u8; 12] {
        let mut response = [0u8; 12];
        response[0..2].copy_from_slice(&tid.to_be_bytes());
        response[2..4].copy_from_slice(&(0u16).to_be_bytes());
        response[4..6].copy_from_slice(&(6u16).to_be_bytes()); // response mbap length
        response[6] = uid;
        response[7] = fc;
        response[8..10].copy_from_slice(&start.to_be_bytes());
        response[10..12].copy_from_slice(&value.to_be_bytes());
        response
    }

    fn multiple_write_request_builder(
        tid: u16,
        uid: u8,
        start: u16,
        values: MultipleWriteValues,
    ) -> Vec<u8> {
        let mut pdu = [0u8; 253];
        let (fc, qty, bc) = match values {
            MultipleWriteValues::Coils(values) => {
                let qty = values.len();
                for (i, &on) in values.iter().enumerate() {
                    if on {
                        pdu[6 + i / 8] |= 1u8 << (i % 8);
                    }
                }

                (0x0f_u8, qty as u16, qty.div_ceil(8))
            }
            MultipleWriteValues::Registers(values) => {
                let qty = values.len();
                for (i, &value) in values.iter().enumerate() {
                    let offset = 6 + i * 2;

                    pdu[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
                }

                (0x10_u8, qty as u16, qty * 2)
            }
        };
        let mut request = Vec::new();
        request.extend_from_slice(&tid.to_be_bytes());
        request.extend_from_slice(&(0u16).to_be_bytes());
        request.extend_from_slice(&(7 + (bc as u16)).to_be_bytes());
        request.push(uid);
        request.push(fc);
        request.extend_from_slice(&start.to_be_bytes());
        request.extend_from_slice(&qty.to_be_bytes());
        request.push(bc as u8);
        request.extend_from_slice(&pdu[6..6 + bc]);
        request
    }

    fn multiple_write_response_builder(
        tid: u16,
        uid: u8,
        fc: u8,
        start: u16,
        qty: u16,
    ) -> [u8; 12] {
        let mut response = [0u8; 12];
        response[0..2].copy_from_slice(&tid.to_be_bytes());
        response[2..4].copy_from_slice(&(0u16).to_be_bytes());
        response[4..6].copy_from_slice(&(6u16).to_be_bytes());
        response[6] = uid;
        response[7] = fc;
        response[8..10].copy_from_slice(&start.to_be_bytes());
        response[10..12].copy_from_slice(&qty.to_be_bytes());

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

    // happy path
    #[test]
    fn single_write_on_returns_expected_bytes() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Coil(true);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = single_write_response_builder(7, 1, 5, 0, 0xff00);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(true);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let result = try_write_single_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), 1);

        server_thread.join().expect("server thread panicked");
    }
    #[test]
    fn single_write_off_return_expected_bytes() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Coil(false);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = single_write_response_builder(7, 1, 5, 0, 0x0000);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(false);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let result = try_write_single_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), 1);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn single_write_register_return_expected_bytes() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Register(0x1234);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = single_write_response_builder(7, 1, 6, 0, 0x1234);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Register(0x1234);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let result = try_write_single_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), 1);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn multiple_write_coils_return_expected_bytes() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([
                true, false, true, false, true, false, true, false, true, false,
            ]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = multiple_write_response_builder(7, 1, 15, 0, expected_qty as u16);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([
            true, false, true, false, true, false, true, false, true, false,
        ]);
        let expected_quantity = coil_values.len();
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let result = try_write_multiple_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), expected_quantity as u16);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn multiple_write_registers_return_expected_bytes() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234, 0x5678]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = multiple_write_response_builder(7, 1, 16, 0, expected_qty as u16);

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234, 0x5678]);
        let expected_quantity = register_values.len();
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let result = try_write_multiple_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), expected_quantity as u16);

        server_thread.join().expect("server thread panicked");
    }

    // multiple write request validation
    #[test]
    fn rejects_empty_write_value() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_values = MultipleWriteValues::Coils(vec![]);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

        assert!(
            matches!(
                expected_error,
                Request(RequestError::InvalidQuantity { .. })
            ),
            "{expected_error:?}"
        );

        let write_values = MultipleWriteValues::Registers(vec![]);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

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
    fn rejects_exceeded_write_value_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_values = MultipleWriteValues::Coils(vec![true; 1969]);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

        assert!(
            matches!(
                expected_error,
                Request(RequestError::InvalidQuantity { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());

        let write_values = MultipleWriteValues::Registers(vec![0x1234; 124]);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

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
    fn rejects_write_address_out_of_range() {
        let (server_addr, server_thread) = create_tcp_test_server(|_| {
            // should rejects before test server process it
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_values = MultipleWriteValues::Coils(vec![true, false]);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 65535,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

        assert!(
            matches!(
                expected_error,
                Request(RequestError::AddressOutOfRange { .. })
            ),
            "{expected_error:?}"
        );
        assert!(expected_error.is_connection_reusable());
        let write_values = MultipleWriteValues::Registers(vec![0x1234, 0x5678]);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 65535,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

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

    // happy path boundary validation on multiple write
    #[test]
    fn accepts_max_coil_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([true; 1968]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = multiple_write_response_builder(7, 1, 15, 0, expected_qty as u16);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([true; 1968]);
        let expected_quantity = coil_values.len();
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let result = try_write_multiple_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), expected_quantity as u16);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn accepts_max_register_qty() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234; 123]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = multiple_write_response_builder(7, 1, 16, 0, expected_qty as u16);

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234; 123]);
        let expected_quantity = register_values.len();
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let result = try_write_multiple_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), expected_quantity as u16);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn accept_max_start_address_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([true]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 65535, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response =
                multiple_write_response_builder(7, 1, 15, 65535, expected_qty as u16);

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([true]);
        let expected_quantity = coil_values.len();
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 65535,
            values: write_values,
        };

        let result = try_write_multiple_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), expected_quantity as u16);

        server_thread.join().expect("server thread panicked");
    }

    #[test]
    fn accepts_max_start_address_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 65535, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response =
                multiple_write_response_builder(7, 1, 16, 65535, expected_qty as u16);

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234]);
        let expected_quantity = register_values.len();
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 65535,
            values: write_values,
        };

        let result = try_write_multiple_once(&mut client_stream, &definition, 7)
            .expect("write should succeed");

        assert_eq!(result.unit_id(), definition.unit_id);
        assert_eq!(result.starting_address(), definition.starting_address);
        assert_eq!(result.quantity(), expected_quantity as u16);

        server_thread.join().expect("server thread panicked");
    }

    // single write response validation
    #[test]
    fn rejects_mismatch_tid_on_single_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Register(0x1234);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = single_write_response_builder(8, 1, 6, 0, 0x1234); // << request tid 7 but response 8
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Register(0x1234);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_tid_on_single_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Coil(true);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = single_write_response_builder(8, 1, 5, 0, 0xff00); // << request tid 7 but response 8
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(true);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_tid_on_multiple_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([
                true, false, true, false, true, false, true, false, true, false,
            ]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = multiple_write_response_builder(8, 1, 15, 0, expected_qty as u16);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([
            true, false, true, false, true, false, true, false, true, false,
        ]);
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_tid_on_multiple_write_registers() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234, 0x5678]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = multiple_write_response_builder(8, 1, 16, 0, expected_qty as u16);

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234, 0x5678]);
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_pid_on_single_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Register(0x1234);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = single_write_response_builder(7, 1, 6, 0, 0x1234);
            actual_response[2..4].copy_from_slice(&(1u16).to_be_bytes()); // change pid to 1 to make it invalid
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Register(0x1234);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_pid_on_single_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Coil(true);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = single_write_response_builder(7, 1, 5, 0, 0xff00);
            actual_response[2..4].copy_from_slice(&(1u16).to_be_bytes()); // change pid to 1 to make it invalid
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(true);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_pid_on_multiple_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([
                true, false, true, false, true, false, true, false, true, false,
            ]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response =
                multiple_write_response_builder(7, 1, 15, 0, expected_qty as u16);
            actual_response[2..4].copy_from_slice(&(1u16).to_be_bytes()); // change pid to 1 to make it invalid
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([
            true, false, true, false, true, false, true, false, true, false,
        ]);
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_pid_on_multiple_write_registers() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234, 0x5678]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response =
                multiple_write_response_builder(7, 1, 16, 0, expected_qty as u16);
            actual_response[2..4].copy_from_slice(&(1u16).to_be_bytes()); // change pid to 1 to make it invalid

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234, 0x5678]);
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_uid_on_single_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Register(0x1234);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = single_write_response_builder(7, 1, 6, 0, 0x1234);
            actual_response[6] = 99; // change uid to 99 to make it invalid
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Register(0x1234);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_uid_on_single_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Coil(true);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = single_write_response_builder(7, 1, 5, 0, 0xff00);
            actual_response[6] = 99; // change uid to 99 to make it invalid
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(true);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_uid_on_multiple_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([
                true, false, true, false, true, false, true, false, true, false,
            ]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response =
                multiple_write_response_builder(7, 1, 15, 0, expected_qty as u16);
            actual_response[6] = 99; // change uid to 99 to make it invalid
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([
            true, false, true, false, true, false, true, false, true, false,
        ]);
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_uid_on_multiple_write_registers() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234, 0x5678]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response =
                multiple_write_response_builder(7, 1, 16, 0, expected_qty as u16);
            actual_response[6] = 99; // change uid to 99 to make it invalid

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234, 0x5678]);
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_fc_on_single_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Register(0x1234);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = single_write_response_builder(7, 1, 6, 0, 0x1234);
            actual_response[7] = 5; // change fc to 5 while actual request is fc 6
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Register(0x1234);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_fc_on_single_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let value = SingleWriteValue::Coil(true);
            let expected_request = single_write_request_builder(7, 1, 0, value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response = single_write_response_builder(7, 1, 5, 0, 0xff00);
            actual_response[7] = 6; // change fc to 6 while actual request is fc 5
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(true);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_fc_on_multiple_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([
                true, false, true, false, true, false, true, false, true, false,
            ]);
            let expected_qty = coil_values.len();
            let values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response =
                multiple_write_response_builder(7, 1, 15, 0, expected_qty as u16);
            actual_response[7] = 16; // change fc to 16 while actual request is fc 15
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([
            true, false, true, false, true, false, true, false, true, false,
        ]);
        let write_values = MultipleWriteValues::Coils(coil_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn rejects_mismatch_fc_on_multiple_write_registers() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234, 0x5678]);
            let expected_qty = register_values.len();
            let values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, values);
            let mut actual_request = vec![0u8; expected_request.len()];

            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let mut actual_response =
                multiple_write_response_builder(7, 1, 16, 0, expected_qty as u16);
            actual_response[7] = 15; // change fc to 15 while actual request is fc 16

            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234, 0x5678]);
        let write_values = MultipleWriteValues::Registers(register_values);

        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    fn returns_device_exception_on_single_write_coil() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let write_value = SingleWriteValue::Coil(true);
            let expected_request = single_write_request_builder(7, 1, 0, write_value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = exception_response_builder(7, 1, 5, 2);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Coil(true);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    #[test]
    fn returns_device_exception_on_single_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let write_value = SingleWriteValue::Register(0x1234);
            let expected_request = single_write_request_builder(7, 1, 0, write_value);
            let mut actual_request = [0u8; 12];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = exception_response_builder(7, 1, 6, 2);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let write_value = SingleWriteValue::Register(0x1234);
        let definition = SingleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            value: write_value,
        };

        let expected_error = try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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

    #[test]
    fn returns_device_exception_on_multiple_write_coils() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let coil_values = Vec::from([true; 1968]);
            let write_values = MultipleWriteValues::Coils(coil_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, write_values);
            let mut actual_request = vec![0u8; expected_request.len()];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = exception_response_builder(7, 1, 15, 2);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let coil_values = Vec::from([true; 1968]);
        let write_values = MultipleWriteValues::Coils(coil_values);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    #[test]
    fn returns_device_exception_on_multiple_write_register() {
        let (server_addr, server_thread) = create_tcp_test_server(|mut server_stream| {
            let register_values = Vec::from([0x1234; 123]);
            let write_values = MultipleWriteValues::Registers(register_values);
            let expected_request = multiple_write_request_builder(7, 1, 0, write_values);
            let mut actual_request = vec![0u8; expected_request.len()];
            server_stream
                .read_exact(&mut actual_request)
                .expect("read request from client");

            assert_eq!(expected_request, actual_request);
            let actual_response = exception_response_builder(7, 1, 16, 2);
            server_stream
                .write_all(&actual_response)
                .expect("should write response");
        });

        let mut client_stream =
            TcpStream::connect(server_addr).expect("should connect to test server");
        let register_values = Vec::from([0x1234; 123]);
        let write_values = MultipleWriteValues::Registers(register_values);
        let definition = MultipleWriteRequestDefinition {
            unit_id: 1,
            starting_address: 0,
            values: write_values,
        };

        let expected_error =
            try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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

    // write response mismatch
    #[test]
    fn rejects_mismatch_response_on_single_write_coil() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let value = SingleWriteValue::Coil(true);
                let expected_request = single_write_request_builder(7, 1, 0, value);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let actual_response = match case {
                    0 => single_write_response_builder(7, 1, 5, 1, 0xff00), // << mismatch start address
                    1 => single_write_response_builder(7, 1, 5, 0, 0x0000), // << mismatch coil response value
                    _ => unreachable!(),
                };
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let write_value = SingleWriteValue::Coil(true);
            let definition = SingleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                value: write_value,
            };

            let expected_error =
                try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();

            assert!(
                matches!(
                    expected_error,
                    Response(ResponseError::WriteResponseMismatch { .. })
                ),
                "{expected_error:?}"
            );
            assert!(expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_mismatch_response_on_single_write_register() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let value = SingleWriteValue::Register(0x1234);
                let expected_request = single_write_request_builder(7, 1, 0, value);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let actual_response = match &case {
                    0 => single_write_response_builder(7, 1, 6, 1, 0x1234), // << mismatch start address
                    1 => single_write_response_builder(7, 1, 6, 0, 0x4321), // << mismatch response register value
                    _ => unreachable!(),
                };
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let write_value = SingleWriteValue::Register(0x1234);
            let definition = SingleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                value: write_value,
            };

            let expected_error =
                try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();

            assert!(
                matches!(
                    expected_error,
                    Response(ResponseError::WriteResponseMismatch { .. })
                ),
                "{expected_error:?}"
            );
            assert!(expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_mismatch_response_on_multiple_write_coils() {
        for case in [1, 0] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let coil_values = Vec::from([true; 16]);
                let write_values = MultipleWriteValues::Coils(coil_values);
                let expected_request = multiple_write_request_builder(7, 1, 0, write_values);
                let mut actual_request = vec![0u8; expected_request.len()];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let actual_response = match case {
                    0 => multiple_write_response_builder(7, 1, 15, 1, 16), // request start address 0 but response 1
                    1 => multiple_write_response_builder(7, 1, 15, 0, 8), // request qty 16 but response 8
                    _ => unreachable!(),
                };
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let coil_values = Vec::from([true; 16]);
            let write_values = MultipleWriteValues::Coils(coil_values);
            let definition = MultipleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                values: write_values,
            };

            let expected_error =
                try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
            assert!(
                matches!(
                    expected_error,
                    Response(ResponseError::WriteResponseMismatch { .. })
                ),
                "{expected_error:?}"
            );
            assert!(expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_mismatch_response_on_multiple_write_registers() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let register_values = Vec::from([0x1234, 0x4321]);
                let write_values = MultipleWriteValues::Registers(register_values);
                let expected_request = multiple_write_request_builder(7, 1, 0, write_values);
                let mut actual_request = vec![0u8; expected_request.len()];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let actual_response = match case {
                    0 => multiple_write_response_builder(7, 1, 16, 1, 2), // request start address 0 but response 1
                    1 => multiple_write_response_builder(7, 1, 16, 0, 4), // request qty 2 but response 4
                    _ => unreachable!(),
                };
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let register_values = Vec::from([0x1234, 0x4321]);
            let write_values = MultipleWriteValues::Registers(register_values);
            let definition = MultipleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                values: write_values,
            };

            let expected_error =
                try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
            assert!(
                matches!(
                    expected_error,
                    Response(ResponseError::WriteResponseMismatch { .. })
                ),
                "{expected_error:?}"
            );
            assert!(expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    // validate frame structure
    #[test]
    fn rejects_invalid_response_mbap_length_on_single_write() {
        for case in [0, 1, 2, 3] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let value = match case {
                    0 | 1 => SingleWriteValue::Coil(true),
                    2 | 3 => SingleWriteValue::Register(0x1234),
                    _ => unreachable!(),
                };
                let expected_request = single_write_request_builder(7, 1, 0, value);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let mut actual_response = single_write_response_builder(7, 1, 5, 0, 0xff00);
                match case {
                    0 | 2 => actual_response[4..6].copy_from_slice(&(2u16).to_be_bytes()), // length = 2
                    1 | 3 => actual_response[4..6].copy_from_slice(&(255u16).to_be_bytes()), // length = 255
                    _ => unreachable!(),
                }

                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");

            let write_value = match case {
                0 | 1 => SingleWriteValue::Coil(true),
                2 | 3 => SingleWriteValue::Register(0x1234),
                _ => unreachable!(),
            };
            let definition = SingleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                value: write_value,
            };

            let expected_error =
                try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();

            assert!(
                matches!(expected_error, Sync(SyncError::InvalidMbapLength { .. })),
                "{expected_error:?}"
            );
            assert!(!expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_invalid_response_mbap_length_on_multiple_write() {
        for case in [0, 1, 2, 3] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let values = match case {
                    0 | 1 => MultipleWriteValues::Coils(vec![true, false]),
                    2 | 3 => MultipleWriteValues::Registers(vec![0x1234, 0x4321]),
                    _ => unreachable!(),
                };
                let expected_request = multiple_write_request_builder(7, 1, 0, values);
                let mut actual_request = vec![0u8; expected_request.len()];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let mut actual_response = match case {
                    0 | 1 => multiple_write_response_builder(7, 1, 15, 0, 2),
                    2 | 3 => multiple_write_response_builder(7, 1, 16, 0, 2),
                    _ => unreachable!(),
                };

                match case {
                    0 | 2 => actual_response[4..6].copy_from_slice(&(2u16).to_be_bytes()),
                    1 | 3 => actual_response[4..6].copy_from_slice(&(255u16).to_be_bytes()),
                    _ => unreachable!(),
                }

                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");

            let write_values = match case {
                0 | 1 => {
                    let coil_values = Vec::from([true, false]);
                    MultipleWriteValues::Coils(coil_values)
                }
                2 | 3 => {
                    let register_values = Vec::from([0x1234, 0x4321]);
                    MultipleWriteValues::Registers(register_values)
                }
                _ => unreachable!(),
            };
            let definition = MultipleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                values: write_values,
            };

            let expected_error =
                try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

            assert!(
                matches!(expected_error, Sync(SyncError::InvalidMbapLength { .. })),
                "{expected_error:?}"
            );
            assert!(!expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_invalid_exception_pdu_len_on_single_write() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let write_value = match case {
                    0 => SingleWriteValue::Coil(true),
                    1 => SingleWriteValue::Register(0x1234),
                    _ => unreachable!(),
                };
                let expected_request = single_write_request_builder(7, 1, 0, write_value);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let mut actual_response = match case {
                    0 => exception_response_builder(7, 1, 5, 2),
                    1 => exception_response_builder(7, 1, 6, 2),
                    _ => unreachable!(),
                };
                actual_response[4..6].copy_from_slice(&(4u16).to_be_bytes());
                actual_response.push(0x99); // bad byte
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");

            let write_value = match case {
                0 => SingleWriteValue::Coil(true),
                1 => SingleWriteValue::Register(0x1234),
                _ => unreachable!(),
            };
            let definition = SingleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                value: write_value,
            };

            let expected_error =
                try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
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
    }

    #[test]
    fn rejects_invalid_exception_pdu_len_on_multiple_write() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let write_values = match case {
                    0 => MultipleWriteValues::Coils(Vec::from([true, false])),
                    1 => MultipleWriteValues::Registers(Vec::from([0x1234, 0x4321])),
                    _ => unreachable!(),
                };
                let expected_request = multiple_write_request_builder(7, 1, 0, write_values);
                let mut actual_request = vec![0u8; expected_request.len()];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let mut actual_response = match case {
                    0 => exception_response_builder(7, 1, 15, 2),
                    1 => exception_response_builder(7, 1, 16, 2),
                    _ => unreachable!(),
                };
                actual_response[4..6].copy_from_slice(&(4u16).to_be_bytes());
                actual_response.push(0x99); // bad byte
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let write_values = match case {
                0 => {
                    let coil_values = Vec::from([true, false]);
                    MultipleWriteValues::Coils(coil_values)
                }
                1 => {
                    let register_values = Vec::from([0x1234, 0x4321]);
                    MultipleWriteValues::Registers(register_values)
                }
                _ => unreachable!(),
            };
            let definition = MultipleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                values: write_values,
            };

            let expected_error =
                try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
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
    }

    #[test]
    fn rejects_invalid_response_pdu_len_on_single_write() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let write_value = match case {
                    0 => SingleWriteValue::Coil(true),
                    1 => SingleWriteValue::Register(0x1234),
                    _ => unreachable!(),
                };
                let expected_request = single_write_request_builder(7, 1, 0, write_value);
                let mut actual_request = [0u8; 12];
                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let valid_response = match case {
                    0 => single_write_response_builder(7, 1, 5, 0, 0xff00),
                    1 => single_write_response_builder(7, 1, 6, 0, 0x1234),
                    _ => unreachable!(),
                };
                let mut actual_response = Vec::from(valid_response);
                actual_response[4..6].copy_from_slice(&(7u16).to_be_bytes()); // change length to 7
                actual_response.push(0x99); // bad byte
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");

            let write_value = match case {
                0 => SingleWriteValue::Coil(true),
                1 => SingleWriteValue::Register(0x1234),
                _ => unreachable!(),
            };
            let definition = SingleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                value: write_value,
            };

            let expected_error =
                try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();
            assert!(
                matches!(
                    expected_error,
                    Response(ResponseError::InvalidWriteResponsePduLen { .. })
                ),
                "{expected_error:?}"
            );
            assert!(expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_invalid_response_pdu_len_on_multiple_write() {
        for case in [0, 1] {
            let (server_addr, server_thread) = create_tcp_test_server(move |mut server_stream| {
                let (write_values, expected_qty) = match case {
                    0 => {
                        let coil_values = Vec::from([true, false]);
                        let expected_qty = coil_values.len();
                        (MultipleWriteValues::Coils(coil_values), expected_qty)
                    }
                    1 => {
                        let register_values = Vec::from([0x1234, 0x4321]);
                        let expected_qty = register_values.len();
                        (
                            MultipleWriteValues::Registers(register_values),
                            expected_qty,
                        )
                    }
                    _ => unreachable!(),
                };
                let expected_request = multiple_write_request_builder(7, 1, 0, write_values);
                let mut actual_request = vec![0u8; expected_request.len()];

                server_stream
                    .read_exact(&mut actual_request)
                    .expect("read request from client");

                assert_eq!(expected_request, actual_request);
                let valid_response = match case {
                    0 => multiple_write_response_builder(7, 1, 15, 0, expected_qty as u16),
                    1 => multiple_write_response_builder(7, 1, 16, 0, expected_qty as u16),
                    _ => unreachable!(),
                };
                let mut actual_response = Vec::from(valid_response);
                actual_response[4..6].copy_from_slice(&(7u16).to_be_bytes()); // change length to 7
                actual_response.push(0x99); // bad byte
                server_stream
                    .write_all(&actual_response)
                    .expect("should write response");
            });

            let mut client_stream =
                TcpStream::connect(server_addr).expect("should connect to test server");
            let write_values = match case {
                0 => {
                    let coil_values = Vec::from([true, false]);
                    MultipleWriteValues::Coils(coil_values)
                }
                1 => {
                    let register_values = Vec::from([0x1234, 0x4321]);
                    MultipleWriteValues::Registers(register_values)
                }
                _ => unreachable!(),
            };

            let definition = MultipleWriteRequestDefinition {
                unit_id: 1,
                starting_address: 0,
                values: write_values,
            };

            let expected_error =
                try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();
            assert!(
                matches!(
                    expected_error,
                    Response(ResponseError::InvalidWriteResponsePduLen { .. })
                ),
                "{expected_error:?}"
            );
            assert!(expected_error.is_connection_reusable());
            server_thread.join().expect("server thread panicked");
        }
    }

    #[test]
    fn rejects_connection_closed_in_flight_on_single_write() {
        for truncated_index in [5, 9] {
            for case in [0, 1] {
                let (server_addr, server_thread) =
                    create_tcp_test_server(move |mut server_stream| {
                        let expected_request = match case {
                            0 => {
                                single_write_request_builder(7, 1, 0, SingleWriteValue::Coil(true))
                            }
                            1 => single_write_request_builder(
                                7,
                                1,
                                0,
                                SingleWriteValue::Register(0x1234),
                            ),
                            _ => unreachable!(),
                        };
                        let mut actual_request = [0u8; 12];
                        server_stream
                            .read_exact(&mut actual_request)
                            .expect("read request from client");

                        assert_eq!(expected_request, actual_request);

                        let actual_response = match case {
                            0 => single_write_response_builder(7, 1, 5, 0, 0xff00),
                            1 => single_write_response_builder(7, 1, 6, 0, 0x1234),
                            _ => unreachable!(),
                        };

                        server_stream
                            .write_all(&actual_response[..truncated_index])
                            .expect("should write response");

                        drop(server_stream);
                    });

                let mut client_stream =
                    TcpStream::connect(server_addr).expect("should connect to test server");
                let definition = SingleWriteRequestDefinition {
                    unit_id: 1,
                    starting_address: 0,
                    value: match case {
                        0 => SingleWriteValue::Coil(true),
                        1 => SingleWriteValue::Register(0x1234),
                        _ => unreachable!(),
                    },
                };

                let expected_error =
                    try_write_single_once(&mut client_stream, &definition, 7).unwrap_err();

                assert!(
                    matches!(expected_error, Io(_)),
                    "cut at {truncated_index}: {expected_error:?}"
                );
                assert!(!expected_error.is_connection_reusable());
                server_thread.join().expect("server thread panicked");
            }
        }
    }

    #[test]
    fn rejects_connection_closed_in_flight_on_multiple_write() {
        for truncated_index in [5, 9] {
            for case in [0, 1] {
                let (server_addr, server_thread) =
                    create_tcp_test_server(move |mut server_stream| {
                        let expected_request = match case {
                            0 => multiple_write_request_builder(
                                7,
                                1,
                                0,
                                MultipleWriteValues::Coils(Vec::from([true, false])),
                            ),
                            1 => multiple_write_request_builder(
                                7,
                                1,
                                0,
                                MultipleWriteValues::Registers(Vec::from([0x1234, 0x4321])),
                            ),
                            _ => unreachable!(),
                        };
                        let mut actual_request = vec![0u8; expected_request.len()];
                        server_stream
                            .read_exact(&mut actual_request)
                            .expect("read request from client");

                        assert_eq!(expected_request, actual_request);

                        let actual_response = match case {
                            0 => multiple_write_response_builder(7, 1, 15, 0, 2),
                            1 => multiple_write_response_builder(7, 1, 16, 0, 2),
                            _ => unreachable!(),
                        };

                        server_stream
                            .write_all(&actual_response[..truncated_index])
                            .expect("should write response");

                        drop(server_stream);
                    });

                let mut client_stream =
                    TcpStream::connect(server_addr).expect("should connect to test server");
                let definition = MultipleWriteRequestDefinition {
                    unit_id: 1,
                    starting_address: 0,
                    values: match case {
                        0 => MultipleWriteValues::Coils(Vec::from([true, false])),
                        1 => MultipleWriteValues::Registers(Vec::from([0x1234, 0x4321])),
                        _ => unreachable!(),
                    },
                };

                let expected_error =
                    try_write_multiple_once(&mut client_stream, &definition, 7).unwrap_err();

                assert!(
                    matches!(expected_error, Io(_)),
                    "cut at {truncated_index}: {expected_error:?}"
                );
                assert!(!expected_error.is_connection_reusable());
                server_thread.join().expect("server thread panicked");
            }
        }
    }
}
