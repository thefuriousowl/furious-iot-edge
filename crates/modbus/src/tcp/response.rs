use crate::function::read_holding_registers::{
    Fc03Response, Fc03ResponseError, decode_fc03_response,
};
use crate::tcp::mbap::MBAP_HEADER_LEN;

#[derive(Debug, PartialEq, Eq)]
enum TcpResponseError {
    HeaderTooShort {
        actual: usize,
        minimum: usize,
    },
    UnsupportedProtocolId {
        actual: u16,
        expected: u16,
    },
    InvalidLength {
        actual: u16,
        minimum: u16,
        maximum: u16,
    },
    AduLengthMismatch {
        actual: usize,
        expected: usize,
    },
    TransactionIdMismatch {
        actual: u16,
        expected: u16,
    },
    UnitIdMismatch {
        actual: u8,
        expected: u8,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum Fc03TcpResponseError {
    Frame(TcpResponseError),
    Pdu(Fc03ResponseError),
}

const MIN_MBAP_LENGTH: u16 = 2;
const MAX_MBAP_LENGTH: u16 = 254;
const MBAP_LENGTH_PREFIX_LEN: usize = 6;
const MODBUS_TCP_PROTOCOL_ID: u16 = 0x0000;
// Validate and correlate a complete response ADU, then borrow its PDU.
fn decode_response_adu(
    adu: &[u8],
    expected_transaction_id: u16,
    expected_unit_id: u8,
) -> Result<&[u8], TcpResponseError> {
    if adu.len() < MBAP_HEADER_LEN {
        return Err(TcpResponseError::HeaderTooShort {
            actual: adu.len(),
            minimum: MBAP_HEADER_LEN,
        });
    }

    let protocol_id = u16::from_be_bytes([adu[2], adu[3]]);
    if protocol_id != MODBUS_TCP_PROTOCOL_ID {
        return Err(TcpResponseError::UnsupportedProtocolId {
            actual: protocol_id,
            expected: MODBUS_TCP_PROTOCOL_ID,
        });
    }

    let length = u16::from_be_bytes([adu[4], adu[5]]);
    if !(MIN_MBAP_LENGTH..=MAX_MBAP_LENGTH).contains(&length) {
        return Err(TcpResponseError::InvalidLength {
            actual: length,
            minimum: MIN_MBAP_LENGTH,
            maximum: MAX_MBAP_LENGTH,
        });
    }

    let expected_total_adu_len = MBAP_LENGTH_PREFIX_LEN + usize::from(length);
    if adu.len() != expected_total_adu_len {
        return Err(TcpResponseError::AduLengthMismatch {
            actual: adu.len(),
            expected: expected_total_adu_len,
        });
    }

    let transaction_id = u16::from_be_bytes([adu[0], adu[1]]);
    if transaction_id != expected_transaction_id {
        return Err(TcpResponseError::TransactionIdMismatch {
            actual: transaction_id,
            expected: expected_transaction_id,
        });
    }

    let unit_id = adu[6];
    if unit_id != expected_unit_id {
        return Err(TcpResponseError::UnitIdMismatch {
            actual: unit_id,
            expected: expected_unit_id,
        });
    }

    Ok(&adu[MBAP_HEADER_LEN..])
}

fn decode_fc03_response_adu(
    adu: &[u8],
    expected_transaction_id: u16,
    expected_unit_id: u8,
    expected_quantity: u16,
) -> Result<Fc03Response, Fc03TcpResponseError> {
    let pdu = decode_response_adu(adu, expected_transaction_id, expected_unit_id)
        .map_err(Fc03TcpResponseError::Frame)?;
    decode_fc03_response(pdu, expected_quantity).map_err(Fc03TcpResponseError::Pdu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::read_holding_registers::Fc03Exception;

    #[test]
    fn accepts_valid_adu() {
        let valid_adu = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];

        let expected = Ok(&valid_adu[MBAP_HEADER_LEN..]);
        let actual = decode_response_adu(&valid_adu, 0x1234, 0x11);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_empty_adu() {
        let empty_adu = [];
        let expected = Err(TcpResponseError::HeaderTooShort {
            actual: empty_adu.len(),
            minimum: MBAP_HEADER_LEN,
        });
        let actual = decode_response_adu(&empty_adu, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_too_short_adu() {
        let truncated_header = [
            0x12, 0x34, // Transaction ID
            0x00, 0x00, // Protocol ID
            0x00, 0x09, // Length
        ];

        let expected = Err(TcpResponseError::HeaderTooShort {
            actual: truncated_header.len(),
            minimum: MBAP_HEADER_LEN,
        });
        let actual = decode_response_adu(&truncated_header, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_raw_length_zero() {
        let adu_with_zero_length = [
            0x12, 0x34, // Transaction ID
            0x00, 0x00, // Protocol ID
            0x00, 0x00, // Length = 0 <--- Invalid
            0x11, // Unit ID
        ];

        let expected = Err(TcpResponseError::InvalidLength {
            actual: 0,
            minimum: MIN_MBAP_LENGTH, // 2
            maximum: MAX_MBAP_LENGTH, // 254
        });

        let actual = decode_response_adu(&adu_with_zero_length, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_raw_length_one() {
        let adu_with_length_one = [
            0x12, 0x34, // Transaction ID
            0x00, 0x00, // Protocol ID
            0x00, 0x01, // Length = 1 <--- Invalid
            0x11, // Unit ID
        ];
        let expected = Err(TcpResponseError::InvalidLength {
            actual: 1,
            minimum: MIN_MBAP_LENGTH,
            maximum: MAX_MBAP_LENGTH,
        });
        let actual = decode_response_adu(&adu_with_length_one, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_raw_length_above_maximum() {
        let adu_with_oversized_length = [
            0x12, 0x34, // Transaction ID
            0x00, 0x00, // Protocol ID
            0x00, 0xff, // Length = 255 <--- Invalid
            0x11, // Unit ID
        ];
        let expected = Err(TcpResponseError::InvalidLength {
            actual: 255,
            minimum: MIN_MBAP_LENGTH,
            maximum: MAX_MBAP_LENGTH,
        });
        let actual = decode_response_adu(&adu_with_oversized_length, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_truncated_adu() {
        let truncated_adu = [
            0x12, 0x34, // Transaction ID
            0x00, 0x00, // Protocol ID
            0x00, 0x09, // Length = 9
            0x11, // Unit ID
            0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            // missing final PDU byte 0x64
        ];

        let expected = Err(TcpResponseError::AduLengthMismatch {
            actual: truncated_adu.len(),
            expected: 15,
        });
        let actual = decode_response_adu(&truncated_adu, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_trailing_byte() {
        let adu_with_trailing_byte = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64, 0xff,
        ];
        let expected = Err(TcpResponseError::AduLengthMismatch {
            actual: adu_with_trailing_byte.len(),
            expected: 15,
        });
        let actual = decode_response_adu(&adu_with_trailing_byte, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn accepts_maximum_valid_frame() {
        let mut maximum_adu = vec![
            0x12, 0x34, // Transaction ID
            0x00, 0x00, // Protocol ID
            0x00, 0xfe, // Length = 254
            0x11, // Unit ID
        ];

        maximum_adu.extend_from_slice(&[0x03; 253]);

        let expected = Ok(&maximum_adu[MBAP_HEADER_LEN..]);
        let actual = decode_response_adu(&maximum_adu, 0x1234, 0x11);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_unsupported_protocol_id() {
        let invalid_protocol_id_adu = [
            0x12, 0x34, 0x00, 0x01, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];

        let expected = Err(TcpResponseError::UnsupportedProtocolId {
            actual: 0x0001,
            expected: 0x0000,
        });
        let actual = decode_response_adu(&invalid_protocol_id_adu, 0x1234, 0x11);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_mismatched_transaction_id() {
        let adu_with_mismatched_transaction_id = [
            0x12, 0x35, 0x00, 0x00, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];
        let expected = Err(TcpResponseError::TransactionIdMismatch {
            actual: 0x1235,
            expected: 0x1234,
        });
        let actual = decode_response_adu(&adu_with_mismatched_transaction_id, 0x1234, 0x11);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_mismatched_unit_id() {
        let adu_with_mismatched_unit_id = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x09, 0x12, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];
        let expected = Err(TcpResponseError::UnitIdMismatch {
            actual: 0x12,
            expected: 0x11,
        });
        let actual = decode_response_adu(&adu_with_mismatched_unit_id, 0x1234, 0x11);
        assert_eq!(actual, expected);
    }

    #[test]
    fn returns_protocol_error_before_transaction_mismatch() {
        let adu_with_invalid_protocol_id_and_transaction_id = [
            0x12, 0x35, 0x00, 0x01, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];
        let expected = Err(TcpResponseError::UnsupportedProtocolId {
            actual: 0x0001,
            expected: MODBUS_TCP_PROTOCOL_ID,
        });
        let actual = decode_response_adu(
            &adu_with_invalid_protocol_id_and_transaction_id,
            0x1234,
            0x11,
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn accepts_valid_fc03_response_adu() {
        let valid_adu = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];
        let expected = Ok(Fc03Response::Registers(vec![0x022b, 0x0000, 0x0064]));
        let actual = decode_fc03_response_adu(&valid_adu, 0x1234, 0x11, 3);
        assert_eq!(actual, expected);
    }

    #[test]
    fn maps_unsupported_protocol_id_to_frame_error() {
        let invalid_protocol_id_adu = [
            0x12, 0x34, 0x00, 0x01, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];
        let expected = Err(Fc03TcpResponseError::Frame(
            TcpResponseError::UnsupportedProtocolId {
                actual: 0x0001,
                expected: 0x0000,
            },
        ));
        let actual = decode_fc03_response_adu(&invalid_protocol_id_adu, 0x1234, 0x11, 3);
        assert_eq!(actual, expected);
    }

    #[test]
    fn maps_transaction_id_mismatch_to_frame_error() {
        let invalid_transaction_id_adu = [
            0x12, 0x35, 0x00, 0x00, 0x00, 0x09, 0x11, 0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00,
            0x64,
        ];
        let expected = Err(Fc03TcpResponseError::Frame(
            TcpResponseError::TransactionIdMismatch {
                actual: 0x1235,
                expected: 0x1234,
            },
        ));
        let actual = decode_fc03_response_adu(&invalid_transaction_id_adu, 0x1234, 0x11, 0x03);
        assert_eq!(actual, expected);
    }

    #[test]
    fn returns_frame_error_before_malformed_pdu_error() {
        let invalid_protocol_id_adu = [0x12, 0x34, 0x00, 0x01, 0x00, 0x02, 0x11, 0x03];

        let expected = Err(Fc03TcpResponseError::Frame(
            TcpResponseError::UnsupportedProtocolId {
                actual: 0x0001,
                expected: 0x0000,
            },
        ));
        let actual = decode_fc03_response_adu(&invalid_protocol_id_adu, 0x1234, 0x11, 0x03);
        assert_eq!(actual, expected);
    }

    #[test]
    fn accepts_valid_modbus_exception_adu() {
        let valid_modbus_exception_adu = [0x12, 0x34, 0x00, 0x00, 0x00, 0x03, 0x11, 0x83, 0x02];
        let expected = Ok(Fc03Response::Exception(Fc03Exception::IllegalDataAddress));
        let actual = decode_fc03_response_adu(&valid_modbus_exception_adu, 0x1234, 0x11, 0x03);
        assert_eq!(actual, expected);
    }

    #[test]
    fn maps_unexpected_function_code_to_pdu_error() {
        let invalid_function_code_adu = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x05, 0x11, 0x04, 0x02, 0x00, 0x01,
        ];
        let expected = Err(Fc03TcpResponseError::Pdu(
            Fc03ResponseError::UnexpectedFunctionCode {
                actual: 0x04,
                expected: 0x03,
            },
        ));
        let actual = decode_fc03_response_adu(&invalid_function_code_adu, 0x1234, 0x11, 0x01);
        assert_eq!(actual, expected);
    }

    #[test]
    fn maps_register_count_mismatch_to_pdu_error() {
        let register_count_mismatch_adu = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x05, 0x11, 0x03, 0x02, 0x00, 0x01,
        ];
        let expected = Err(Fc03TcpResponseError::Pdu(
            Fc03ResponseError::RegisterCountMismatch {
                actual: 1,
                expected: 2,
            },
        ));
        let actual = decode_fc03_response_adu(&register_count_mismatch_adu, 0x1234, 0x11, 2);
        assert_eq!(actual, expected);
    }
}
