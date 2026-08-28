#[derive(Debug, PartialEq, Eq)]
enum MbapError {
    EmptyPdu,
    PduTooLarge { actual: usize, maximum: usize },
}

const MBAP_HEADER_LEN: usize = 7;

fn encode_mbap_header(transaction_id: u16, length: u16, unit_id: u8) -> [u8; MBAP_HEADER_LEN] {
    let mut header = [0_u8; MBAP_HEADER_LEN];
    let transaction_bytes = transaction_id.to_be_bytes();
    header[0..2].copy_from_slice(&transaction_bytes);
    let length_bytes = length.to_be_bytes();
    header[4..6].copy_from_slice(&length_bytes);
    header[6] = unit_id;

    header
}

const MIN_PDU_LEN: usize = 1;
const MAX_PDU_LEN: usize = 253;
const UNIT_IDENTIFIER_SIZE: usize = 1;

fn pdu_len_to_mbap_len(pdu_len: usize) -> Result<u16, MbapError> {
    if pdu_len < MIN_PDU_LEN {
        return Err(MbapError::EmptyPdu);
    }

    if pdu_len > MAX_PDU_LEN {
        return Err(MbapError::PduTooLarge {
            actual: pdu_len,
            maximum: MAX_PDU_LEN,
        });
    }

    // add sizeof unit identifier (1 byte)
    let mbap_len_usize =
        pdu_len
            .checked_add(UNIT_IDENTIFIER_SIZE)
            .ok_or(MbapError::PduTooLarge {
                actual: pdu_len,
                maximum: MAX_PDU_LEN,
            })?;

    // Checked Conversion is u16
    u16::try_from(mbap_len_usize).map_err(|_| MbapError::PduTooLarge {
        actual: pdu_len,
        maximum: MAX_PDU_LEN,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_valid_mbap_header() {
        let transaction_id = 0x1234;
        let length = 0x0006;
        let unit_id = 0x11;

        let actual = encode_mbap_header(transaction_id, length, unit_id);
        let expected: [u8; 7] = [0x12, 0x34, 0x00, 0x00, 0x00, 0x06, 0x11];

        assert_eq!(actual, expected);
    }

    #[test]
    fn converts_valid_pdu_lengths() {
        assert_eq!(pdu_len_to_mbap_len(1), Ok(2));
        assert_eq!(pdu_len_to_mbap_len(5), Ok(6));
        assert_eq!(pdu_len_to_mbap_len(253), Ok(254));
    }

    #[test]
    fn rejects_invalid_pdu_lengths() {
        assert_eq!(pdu_len_to_mbap_len(0), Err(MbapError::EmptyPdu));
        assert_eq!(
            pdu_len_to_mbap_len(254),
            Err(MbapError::PduTooLarge {
                actual: 254,
                maximum: MAX_PDU_LEN
            })
        )
    }
}
