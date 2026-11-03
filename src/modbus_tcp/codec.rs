use crate::modbus_tcp::read::{MBAP_HEADER_LEN, MODBUS_TCP_PID};

pub(crate) fn assemble_read_request_pdu(
    function_code: u8,
    starting_address: u16,
    quantity: u16,
) -> [u8; 5] {
    let mut request_pdu = [0u8; 5];
    request_pdu[0] = function_code;
    request_pdu[1..3].copy_from_slice(&starting_address.to_be_bytes());
    request_pdu[3..5].copy_from_slice(&quantity.to_be_bytes());
    request_pdu
}

pub(crate) fn assemble_request_mbap(
    request_tid: u16,
    unit_id: u8,
    request_mbap_length: u16,
) -> [u8; MBAP_HEADER_LEN] {
    let mut request_mbap = [0u8; MBAP_HEADER_LEN];
    request_mbap[0..2].copy_from_slice(&request_tid.to_be_bytes());
    request_mbap[2..4].copy_from_slice(&MODBUS_TCP_PID.to_be_bytes());
    request_mbap[4..6].copy_from_slice(&request_mbap_length.to_be_bytes());
    request_mbap[6] = unit_id;

    request_mbap
}

pub(crate) fn assemble_single_write_request_pdu(
    function_code: u8,
    starting_address: u16,
    wire_value: u16,
) -> [u8; 5] {
    let mut request_pdu = [0u8; 5];
    request_pdu[0] = function_code;
    request_pdu[1..3].copy_from_slice(&starting_address.to_be_bytes());
    request_pdu[3..5].copy_from_slice(&wire_value.to_be_bytes());
    request_pdu
}

pub(crate) fn assemble_request_adu(
    request_mbap: [u8; MBAP_HEADER_LEN],
    request_pdu: [u8; 5],
) -> [u8; 12] {
    let mut request_adu = [0u8; 12];
    request_adu[..MBAP_HEADER_LEN].copy_from_slice(&request_mbap);
    request_adu[MBAP_HEADER_LEN..].copy_from_slice(&request_pdu);
    request_adu
}
