use super::{error::ValidationError, helper::ensure};
use crate::modbus_tcp::{
    MultipleWriteRequestDefinition,
    codec::{assemble_request_adu, assemble_request_mbap, assemble_single_write_request_pdu},
    definition::{MultipleWriteValues, SingleWriteRequestDefinition, SingleWriteValue},
    error::ModbusTcpError,
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
        ValidationError::InvalidMbapLength {
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
        ValidationError::TransactionIdMismatch {
            expected: request_tid,
            found: response_tid,
        },
    )?;

    // Read and validate response protocol ID
    let response_pid = u16::from_be_bytes([response_mbap[2], response_mbap[3]]);
    ensure(
        response_pid == MODBUS_TCP_PID,
        ValidationError::InvalidProtocolId {
            expected: MODBUS_TCP_PID,
            found: response_pid,
        },
    )?;

    // Read and validate unit ID
    let response_uid = response_mbap[6];
    ensure(
        response_uid == definition.unit_id,
        ValidationError::UnitIdMismatch {
            expected: definition.unit_id,
            found: response_uid,
        },
    )?;

    // Read and validate function code
    let response_fc = response_pdu[0];

    if response_fc == (function_code | 0x80) {
        ensure(
            response_pdu_len == 2,
            ValidationError::InvalidExceptionPduLen {
                expected: 2,
                found: response_pdu_len,
            },
        )?;

        return Err(ModbusTcpError::Exception(response_pdu[1].into()));
    }

    ensure(
        response_fc == function_code,
        ValidationError::FunctionCodeMismatch {
            request: function_code,
            response: response_fc,
        },
    )?;

    ensure(
        response_pdu_len == 5,
        ValidationError::InvalidWriteResponsePduLen {
            expected: 5,
            found: response_pdu_len,
        },
    )?;

    ensure(
        response_pdu[..response_pdu_len] == request_pdu,
        ValidationError::WriteResponseMismatch,
    )?;

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
                ValidationError::InvalidQuantity {
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
                ValidationError::InvalidQuantity {
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
        ValidationError::AddressOutOfRange {
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
        ValidationError::InvalidMbapLength {
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
        ValidationError::TransactionIdMismatch {
            expected: request_tid,
            found: response_tid,
        },
    )?;

    // Read and validate response protocol ID
    let response_pid = u16::from_be_bytes([response_mbap[2], response_mbap[3]]);
    ensure(
        response_pid == MODBUS_TCP_PID,
        ValidationError::InvalidProtocolId {
            expected: MODBUS_TCP_PID,
            found: response_pid,
        },
    )?;

    // Read and validate unit ID
    let response_uid = response_mbap[6];
    ensure(
        definition.unit_id == response_uid,
        ValidationError::UnitIdMismatch {
            expected: definition.unit_id,
            found: response_uid,
        },
    )?;

    // Read and validate function code
    let response_fc = response_pdu[0];
    if response_fc == (function_code | 0x80) {
        ensure(
            response_pdu_len == 2,
            ValidationError::InvalidExceptionPduLen {
                expected: 2,
                found: response_pdu_len,
            },
        )?;

        return Err(ModbusTcpError::Exception(response_pdu[1].into()));
    }

    ensure(
        response_fc == function_code,
        ValidationError::FunctionCodeMismatch {
            request: function_code,
            response: response_fc,
        },
    )?;

    ensure(
        response_pdu_len == 5,
        ValidationError::InvalidWriteResponsePduLen {
            expected: 5,
            found: response_pdu_len,
        },
    )?;

    ensure(
        response_pdu[..response_pdu_len] == request_pdu[..5],
        ValidationError::WriteResponseMismatch,
    )?;

    Ok(WriteAck::new(
        response_uid,
        definition.starting_address,
        quantity,
    ))
}
