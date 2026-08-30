#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fc03Error {
    QuantityOutOfRange {
        actual: u16,
        minimum: u16,
        maximum: u16,
    },
    AddressRangeOverflow {
        start_address: u16,
        quantity: u16,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fc03ResponseError {
    OddByteLength {
        actual: u8,
    },
    RegisterCountMismatch {
        actual: u16,
        expected: u16,
    },
    PduTooShort {
        actual: usize,
        minimum: usize,
    },
    UnexpectedFunctionCode {
        actual: u8,
        expected: u8,
    },
    ExpectedQuantityOutOfRange {
        actual: u16,
        minimum: u16,
        maximum: u16,
    },
    PduLengthMismatch {
        actual: usize,
        expected: usize,
    },
    UnknownExceptionCode {
        actual: u8,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fc03Response {
    Registers(Vec<u16>),
    Exception(Fc03Exception),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fc03Exception {
    IllegalFunction,
    IllegalDataAddress,
    IllegalDataValue,
    ServerDeviceFailure,
}
pub(crate) struct ReadHoldingRegistersRequest {
    start_address: u16,
    quantity: u16,
}

const MINIMUM_QUANTITY: u16 = 1;
const MAXIMUM_QUANTITY: u16 = 125;
const FC03_EXCEPTION_FUNCTION_CODE: u8 = 0x83;
const FC03_FUNCTION_CODE: u8 = 0x03;
const FC03_RESPONSE_PREFIX_LENGTH: usize = 2;
const FC03_EXCEPTION_PDU_LENGTH: usize = 2;

impl ReadHoldingRegistersRequest {
    pub(crate) fn new(start_address: u16, quantity: u16) -> Result<Self, Fc03Error> {
        if !(MINIMUM_QUANTITY..=MAXIMUM_QUANTITY).contains(&quantity) {
            return Err(Fc03Error::QuantityOutOfRange {
                actual: quantity,
                minimum: MINIMUM_QUANTITY,
                maximum: MAXIMUM_QUANTITY,
            });
        }

        let offset = quantity
            .checked_sub(1)
            .ok_or(Fc03Error::QuantityOutOfRange {
                actual: quantity,
                minimum: MINIMUM_QUANTITY,
                maximum: MAXIMUM_QUANTITY,
            })?;

        start_address
            .checked_add(offset)
            .ok_or(Fc03Error::AddressRangeOverflow {
                start_address,
                quantity,
            })?;

        Ok(Self {
            start_address,
            quantity,
        })
    }

    pub(crate) fn encode(&self) -> [u8; 5] {
        let [start_hi, start_lo] = self.start_address.to_be_bytes();
        let [qty_hi, qty_lo] = self.quantity.to_be_bytes();
        [FC03_FUNCTION_CODE, start_hi, start_lo, qty_hi, qty_lo]
    }
}

pub(crate) fn decode_fc03_response(
    pdu: &[u8],
    expected_quantity: u16,
) -> Result<Fc03Response, Fc03ResponseError> {
    if pdu.first() == Some(&FC03_EXCEPTION_FUNCTION_CODE) {
        let exception_decoded = decode_fc03_exception_response(pdu);
        exception_decoded.map(Fc03Response::Exception)
    } else {
        let decoded = decode_fc03_success_response(pdu, expected_quantity);
        decoded.map(Fc03Response::Registers)
    }
}

fn decode_fc03_success_response(
    pdu: &[u8],
    expected_quantity: u16,
) -> Result<Vec<u16>, Fc03ResponseError> {
    if !(MINIMUM_QUANTITY..=MAXIMUM_QUANTITY).contains(&expected_quantity) {
        return Err(Fc03ResponseError::ExpectedQuantityOutOfRange {
            actual: expected_quantity,
            minimum: MINIMUM_QUANTITY,
            maximum: MAXIMUM_QUANTITY,
        });
    }

    if pdu.len() < FC03_RESPONSE_PREFIX_LENGTH {
        return Err(Fc03ResponseError::PduTooShort {
            actual: pdu.len(),
            minimum: FC03_RESPONSE_PREFIX_LENGTH,
        });
    }

    if pdu[0] != FC03_FUNCTION_CODE {
        return Err(Fc03ResponseError::UnexpectedFunctionCode {
            actual: pdu[0],
            expected: FC03_FUNCTION_CODE,
        });
    }

    let data_length = usize::from(pdu[1]);
    let expected_pdu_length = FC03_RESPONSE_PREFIX_LENGTH + data_length;

    if pdu.len() != expected_pdu_length {
        return Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: expected_pdu_length,
        });
    }

    if !data_length.is_multiple_of(2) {
        return Err(Fc03ResponseError::OddByteLength { actual: pdu[1] });
    }

    let actual_quantity = u16::from(pdu[1]) / 2;

    if actual_quantity != expected_quantity {
        return Err(Fc03ResponseError::RegisterCountMismatch {
            actual: actual_quantity,
            expected: expected_quantity,
        });
    }

    let mut registers = Vec::with_capacity(usize::from(expected_quantity));
    let (pairs, _remainder) = pdu[FC03_RESPONSE_PREFIX_LENGTH..].as_chunks::<2>();

    for pair in pairs {
        let value = u16::from_be_bytes([pair[0], pair[1]]);
        registers.push(value);
    }

    Ok(registers)
}

fn decode_fc03_exception_response(pdu: &[u8]) -> Result<Fc03Exception, Fc03ResponseError> {
    if pdu.len() != FC03_EXCEPTION_PDU_LENGTH {
        return Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: FC03_EXCEPTION_PDU_LENGTH,
        });
    }

    if pdu[0] != FC03_EXCEPTION_FUNCTION_CODE {
        return Err(Fc03ResponseError::UnexpectedFunctionCode {
            actual: pdu[0],
            expected: FC03_EXCEPTION_FUNCTION_CODE,
        });
    }

    match pdu[1] {
        0x01 => Ok(Fc03Exception::IllegalFunction),
        0x02 => Ok(Fc03Exception::IllegalDataAddress),
        0x03 => Ok(Fc03Exception::IllegalDataValue),
        0x04 => Ok(Fc03Exception::ServerDeviceFailure),
        actual => Err(Fc03ResponseError::UnknownExceptionCode { actual }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constructs_valid_request() -> Result<(), Fc03Error> {
        let request = ReadHoldingRegistersRequest::new(0x006b, 3)?;
        assert_eq!(request.start_address, 0x006b);
        assert_eq!(request.quantity, 3);
        assert_eq!(request.encode(), [0x03, 0x00, 0x6b, 0x00, 0x03]);

        Ok(())
    }

    #[test]
    fn rejects_quantities_outside_valid_range() {
        assert_eq!(
            ReadHoldingRegistersRequest::new(0, 0).err(),
            Some(Fc03Error::QuantityOutOfRange {
                actual: 0,
                minimum: MINIMUM_QUANTITY,
                maximum: MAXIMUM_QUANTITY,
            })
        );

        assert_eq!(
            ReadHoldingRegistersRequest::new(0, 126).err(),
            Some(Fc03Error::QuantityOutOfRange {
                actual: 126,
                minimum: MINIMUM_QUANTITY,
                maximum: MAXIMUM_QUANTITY,
            })
        );
    }

    #[test]
    fn accepts_minimum_and_maximum_quantities() {
        assert!(ReadHoldingRegistersRequest::new(0, MINIMUM_QUANTITY).is_ok());
        assert!(ReadHoldingRegistersRequest::new(0, MAXIMUM_QUANTITY).is_ok());
    }

    #[test]
    fn validates_last_address_in_requested_range() {
        assert_eq!(
            ReadHoldingRegistersRequest::new(u16::MAX, 2).err(),
            Some(Fc03Error::AddressRangeOverflow {
                start_address: u16::MAX,
                quantity: 2,
            })
        );
        assert!(ReadHoldingRegistersRequest::new(u16::MAX, 1).is_ok());
    }

    #[test]
    fn accepts_valid_fc03_response() {
        let pdu: [u8; 8] = [0x03, 0x06, 0x02, 0x2b, 0x00, 0x00, 0x00, 0x64];
        let expected: Result<Vec<u16>, Fc03ResponseError> = Ok(vec![0x022b, 0x0000, 0x0064]);
        let actual = decode_fc03_success_response(&pdu, 3);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_empty_pdu() {
        let pdu = [];
        let expected = Err(Fc03ResponseError::PduTooShort {
            actual: pdu.len(),
            minimum: FC03_RESPONSE_PREFIX_LENGTH,
        });
        let actual = decode_fc03_success_response(&pdu, 2);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_one_byte_pdu() {
        let pdu = [0x00];
        let expected = Err(Fc03ResponseError::PduTooShort {
            actual: pdu.len(),
            minimum: FC03_RESPONSE_PREFIX_LENGTH,
        });
        let actual = decode_fc03_success_response(&pdu, 2);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_unexpected_fc() {
        let pdu = [0x04, 0x02, 0x00, 0x01];
        let expected = Err(Fc03ResponseError::UnexpectedFunctionCode {
            actual: pdu[0],
            expected: FC03_FUNCTION_CODE,
        });
        let actual = decode_fc03_success_response(&pdu, 1);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_zero_quantity() {
        let pdu: [u8; 2] = [0x03, 0x00];
        let expected_quantity = 0;
        let expected: Result<_, Fc03ResponseError> =
            Err(Fc03ResponseError::ExpectedQuantityOutOfRange {
                actual: expected_quantity,
                minimum: MINIMUM_QUANTITY,
                maximum: MAXIMUM_QUANTITY,
            });
        let actual = decode_fc03_success_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_upper_bound_qty_exceed() {
        let pdu = [0x03, 0x02, 0x00, 0x01];
        let expected_quantity = 126;
        let expected = Err(Fc03ResponseError::ExpectedQuantityOutOfRange {
            actual: expected_quantity,
            minimum: MINIMUM_QUANTITY,
            maximum: MAXIMUM_QUANTITY,
        });
        let actual = decode_fc03_success_response(&pdu, expected_quantity);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_odd_bytes_count() {
        let pdu = [0x03, 0x03, 0x00, 0x01, 0x02];
        let expected_quantity = 1;
        let expected = Err(Fc03ResponseError::OddByteLength { actual: pdu[1] });
        let actual = decode_fc03_success_response(&pdu, expected_quantity);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_pdu_length_mismatch() {
        let pdu = [0x03, 0x06, 0x02, 0x2b];
        let expected_quantity: u16 = 3;
        let expected = Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: FC03_RESPONSE_PREFIX_LENGTH + usize::from(pdu[1]),
        });
        let actual = decode_fc03_success_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_trailing_byte_after_declared_data() {
        let pdu = [0x03, 0x02, 0x00, 0x01, 0xff];
        let expected_quantity = 1;
        let expected = Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: FC03_RESPONSE_PREFIX_LENGTH + usize::from(pdu[1]),
        });
        let actual = decode_fc03_success_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_pdu_register_count_mismatch() {
        let pdu = [0x03, 0x02, 0x00, 0x01];
        let expected_quantity = 2;
        let actual = decode_fc03_success_response(&pdu, expected_quantity);
        let expected = Err(Fc03ResponseError::RegisterCountMismatch {
            actual: 1,
            expected: expected_quantity,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn accepts_minimum_response_quantity() {
        let pdu = [0x03, 0x02, 0x12, 0x34];
        let expected_quantity = 1;
        let actual = decode_fc03_success_response(&pdu, expected_quantity);
        let expected = Ok(vec![0x1234]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn accepts_maximum_response_quantity() {
        let mut pdu: Vec<u8> = vec![0x03, 0xfa];
        for _ in 0..MAXIMUM_QUANTITY {
            pdu.push(0x12);
            pdu.push(0x34);
        }

        let expected = Ok(vec![0x1234; usize::from(MAXIMUM_QUANTITY)]);

        assert_eq!(
            decode_fc03_success_response(&pdu, MAXIMUM_QUANTITY),
            expected
        );
    }

    #[test]
    fn decodes_illegal_data_address_exception() {
        let pdu = [0x83, 0x02];
        let expected = Ok(Fc03Exception::IllegalDataAddress);
        let actual = decode_fc03_exception_response(&pdu);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_empty_exception_pdu() {
        let pdu = [];
        let expected = Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: FC03_EXCEPTION_PDU_LENGTH,
        });
        let actual = decode_fc03_exception_response(&pdu);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_one_byte_exception_pdu() {
        let pdu = [0x83];
        let expected = Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: FC03_EXCEPTION_PDU_LENGTH,
        });
        let actual = decode_fc03_exception_response(&pdu);

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_trailing_byte_in_exception_pdu() {
        let pdu = [0x83, 0x02, 0xff];
        let expected = Err(Fc03ResponseError::PduLengthMismatch {
            actual: pdu.len(),
            expected: FC03_EXCEPTION_PDU_LENGTH,
        });
        let actual = decode_fc03_exception_response(&pdu);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_wrong_exception_function_code() {
        let pdu = [0x84, 0x02];
        let expected = Err(Fc03ResponseError::UnexpectedFunctionCode {
            actual: pdu[0],
            expected: FC03_EXCEPTION_FUNCTION_CODE,
        });
        let actual = decode_fc03_exception_response(&pdu);

        assert_eq!(actual, expected);
    }

    #[test]
    fn decodes_illegal_function_exception() {
        let pdu = [0x83, 0x01];
        let expected = Ok(Fc03Exception::IllegalFunction);
        let actual = decode_fc03_exception_response(&pdu);
        assert_eq!(actual, expected);
    }

    #[test]
    fn decodes_illegal_data_value_exception() {
        let pdu = [0x83, 0x03];
        let expected = Ok(Fc03Exception::IllegalDataValue);
        let actual = decode_fc03_exception_response(&pdu);
        assert_eq!(actual, expected);
    }

    #[test]
    fn decodes_server_device_failure_exception() {
        let pdu = [0x83, 0x04];
        let expected = Ok(Fc03Exception::ServerDeviceFailure);
        let actual = decode_fc03_exception_response(&pdu);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_unknown_exception_code() {
        let pdu = [0x83, 0x07];
        let expected = Err(Fc03ResponseError::UnknownExceptionCode { actual: pdu[1] });
        let actual = decode_fc03_exception_response(&pdu);
        assert_eq!(actual, expected);
    }

    #[test]
    fn decodes_success_response_through_outer_decoder() {
        let pdu = [0x03, 0x02, 0x12, 0x34];
        let expected_quantity = 1;
        let expected = Ok(Fc03Response::Registers(vec![0x1234]));
        let actual = decode_fc03_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn decodes_exception_response_through_outer_decoder() {
        let pdu = [0x83, 0x02];
        let expected_quantity = 1;
        let expected = Ok(Fc03Response::Exception(Fc03Exception::IllegalDataAddress));
        let actual = decode_fc03_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_empty_pdu_through_outer_decoder() {
        let pdu = [];
        let expected_quantity = 1;
        let expected = Err(Fc03ResponseError::PduTooShort {
            actual: 0,
            minimum: FC03_RESPONSE_PREFIX_LENGTH,
        });
        let actual = decode_fc03_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_truncated_exception_through_outer_decoder() {
        let pdu = [0x83];
        let expected_quantity = 1;
        let expected = Err(Fc03ResponseError::PduLengthMismatch {
            actual: 1,
            expected: FC03_EXCEPTION_PDU_LENGTH,
        });
        let actual = decode_fc03_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_unrelated_function_code_through_outer_decoder() {
        let pdu = [0x84, 0x02];
        let expected_quantity = 1;
        let expected = Err(Fc03ResponseError::UnexpectedFunctionCode {
            actual: 0x84,
            expected: FC03_FUNCTION_CODE,
        });
        let actual = decode_fc03_response(&pdu, expected_quantity);
        assert_eq!(actual, expected);
    }
}
