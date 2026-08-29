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

pub(crate) struct ReadHoldingRegistersRequest {
    start_address: u16,
    quantity: u16,
}

const MINIMUM_QUANTITY: u16 = 1;
const MAXIMUM_QUANTITY: u16 = 125;
const FC03_FUNCTION_CODE: u8 = 0x03;

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
}
