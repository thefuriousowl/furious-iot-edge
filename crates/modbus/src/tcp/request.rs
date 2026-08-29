use crate::function::read_holding_registers::{Fc03Error, ReadHoldingRegistersRequest};

use super::mbap::{MbapError, MbapHeader};
#[derive(Debug, PartialEq, Eq)]
enum TcpRequestError {
    Request(Fc03Error),
    Header(MbapError),
}

// Combined MBAP Header + PDU into 12 bytes ADU
fn assemble_fc03_request(
    transaction_id: u16,
    unit_id: u8,
    start_address: u16,
    quantity: u16,
) -> Result<[u8; 12], TcpRequestError> {
    let pdu = ReadHoldingRegistersRequest::new(start_address, quantity)
        .map_err(TcpRequestError::Request)?
        .encode();
    let mbap_header = MbapHeader::new(transaction_id, unit_id, pdu.len())
        .map_err(TcpRequestError::Header)?
        .encode();
    let mut adu: [u8; 12] = [0u8; 12];
    adu[0..7].copy_from_slice(&mbap_header);
    adu[7..12].copy_from_slice(&pdu);
    Ok(adu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_exact_fc03_tcp_request() -> Result<(), TcpRequestError> {
        let expected: [u8; 12] = [
            0x12, 0x34, 0x00, 0x00, 0x00, 0x06, 0x11, 0x03, 0x00, 0x6b, 0x00, 0x03,
        ];
        let actual = assemble_fc03_request(0x1234, 0x11, 0x006b, 0x03)?;
        assert_eq!(actual, expected);

        Ok(())
    }

    #[test]
    fn rejects_empty_quantity() {
        let expected = Some(TcpRequestError::Request(Fc03Error::QuantityOutOfRange {
            actual: 0,
            minimum: 1,
            maximum: 125,
        }));
        let actual = assemble_fc03_request(0x1234, 0x11, 0x00, 0x00).err();

        assert_eq!(actual, expected);
    }

    #[test]
    fn rejects_address_range_overflow() {
        let expected = Some(TcpRequestError::Request(Fc03Error::AddressRangeOverflow {
            start_address: u16::MAX,
            quantity: 2,
        }));
        let actual = assemble_fc03_request(0x1234, 0x11, u16::MAX, 2).err();

        assert_eq!(actual, expected);
    }
}
