use std::{
    io::{Read, Write},
    net::TcpStream,
};

use crate::modbus_tcp::{
    codec::{assemble_read_request_pdu, assemble_request_adu, assemble_request_mbap},
    definition::ReadRequestDefinition,
    error::{
        ModbusTcpError::{self, Validation},
        ValidationError,
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
            return Err(Validation(ValidationError::UnexpectedFunctionCode {
                found: fc,
            }));
        }
    };

    ensure(
        (1..=max_quantity).contains(&definition.quantity),
        ValidationError::InvalidQuantity {
            min: 1,
            max: max_quantity,
            found: definition.quantity,
        },
    )?;

    let check_range_add = u32::from(definition.starting_address) + u32::from(definition.quantity);

    if check_range_add > 65536 {
        return Err(Validation(ValidationError::AddressOutOfRange {
            min: 1,
            max: 65535,
            found: check_range_add,
        }));
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
    let exception_fc = definition.function_code | 0x80;
    if response_fc == exception_fc {
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
        response_fc == definition.function_code,
        ValidationError::FunctionCodeMismatch {
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
        ValidationError::ByteCountMismatched {
            expected: expected_byte_count as u16,
            found: u16::from(response_byte_count),
        },
    )?;

    ensure(
        data_len == usize::from(response_byte_count),
        ValidationError::ActualDataLenMismatchByteCount {
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
