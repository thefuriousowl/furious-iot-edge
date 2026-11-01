use std::{
    io::{Read, Write},
    net::TcpStream,
};

const MBAP_HEADER_LEN: usize = 7;
fn main() {
    let mut client_stream = TcpStream::connect("127.0.0.1:10502").unwrap();

    // // FC16 PDU
    // let request_fc: u8 = 0x10; // DEC 16
    // let request_starting_address: u16 = 0;
    // let request_write_register_values = [0x1234u16; 123]; // For FC16
    // let request_quantity: u16 = request_write_register_values.len() as u16;
    // assert!((1..=123).contains(&request_quantity));
    // assert!((request_starting_address as u32) + (request_quantity as u32) <= 65536);
    // let request_byte_count: u16 = request_quantity * 2;
    // let request_pdu_len: usize = 6 + (request_byte_count as usize); // <=253 bytes
    // let request_adu_len: usize = MBAP_HEADER_LEN + request_pdu_len;

    ///////////////

    // FC15 PDU
    let request_fc: u8 = 0x0f; // DEC 15
    let request_starting_address: u16 = 0;
    // let request_write_bit_values = [false; 1968]; // For FC15
    let request_write_bit_values = [
        true, false, true, true, false, false, false, true, true, false,
    ];

    let request_quantity: u16 = request_write_bit_values.len() as u16;
    assert!((1..=1968).contains(&request_quantity));
    assert!((request_starting_address as u32) + (request_quantity as u32) <= 65536);
    let request_byte_count: u16 = request_quantity.div_ceil(8);
    let request_pdu_len: usize = 6 + (request_byte_count as usize); // <=253 bytes
    let request_adu_len: usize = MBAP_HEADER_LEN + request_pdu_len;

    //////////////

    let mut request_pdu_buffer = [0u8; 253];
    let actual_request_pdu = &mut request_pdu_buffer[..request_pdu_len];
    actual_request_pdu[0] = request_fc;
    actual_request_pdu[1..3].copy_from_slice(&request_starting_address.to_be_bytes());
    actual_request_pdu[3..5].copy_from_slice(&request_quantity.to_be_bytes());
    actual_request_pdu[5] = request_byte_count as u8;

    // // FC16
    // let mut start = 6;
    // for value in request_write_register_values {
    //     actual_request_pdu[start..start + 2].copy_from_slice(&value.to_be_bytes());
    //     start += 2;
    // }

    // FC15
    for (i, value) in request_write_bit_values.iter().enumerate() {
        if *value {
            actual_request_pdu[6 + i / 8] |= 1 << (i % 8);
        }
    }

    // println!("{:x?}", actual_request_pdu);

    // MBAP
    let request_mbap_length = 1 + (request_pdu_len as u16);
    let request_tid: u16 = 1;
    let request_pid: u16 = 0;
    let request_uid: u8 = 1;
    let mut request_mbap = [0u8; MBAP_HEADER_LEN];
    request_mbap[0..2].copy_from_slice(&request_tid.to_be_bytes());
    request_mbap[2..4].copy_from_slice(&request_pid.to_be_bytes());
    request_mbap[4..6].copy_from_slice(&request_mbap_length.to_be_bytes());
    request_mbap[6] = request_uid;

    let mut request_adu = [0u8; 260];
    request_adu[..MBAP_HEADER_LEN].copy_from_slice(&request_mbap);
    request_adu[MBAP_HEADER_LEN..request_adu_len].copy_from_slice(actual_request_pdu);

    client_stream
        .write_all(&request_adu[..request_adu_len])
        .unwrap();

    // Modbus Response
    let mut response_mbap = [0u8; MBAP_HEADER_LEN];
    client_stream.read_exact(&mut response_mbap).unwrap();

    let response_length = u16::from_be_bytes([response_mbap[4], response_mbap[5]]);
    assert!((3..=254).contains(&response_length));

    let response_pdu_len = usize::from(response_length - 1);

    let mut response_pdu = [0u8; 253];

    client_stream
        .read_exact(&mut response_pdu[..response_pdu_len])
        .unwrap();

    // Read and validate response transaction ID
    let response_tid = u16::from_be_bytes([response_mbap[0], response_mbap[1]]);
    assert_eq!(response_tid, request_tid, "Transaction ID mismatch");

    // Read and validate response protocol ID
    let response_pid = u16::from_be_bytes([response_mbap[2], response_mbap[3]]);
    assert_eq!(response_pid, 0, "Invalid protocol ID");

    // Read and validate unit ID
    let response_uid = response_mbap[6];
    assert_eq!(response_uid, request_uid, "Unit ID mismatch");

    // Read and validate function code
    let response_fc = response_pdu[0];
    assert!(response_fc == request_fc || response_fc == (request_fc | 0x80));

    // success or exception
    match response_fc {
        // success route
        0x10 | 0x0f => {
            assert_eq!(response_pdu_len, 5);
            assert_eq!(&response_pdu[..5], &actual_request_pdu[..5]);

            match response_fc {
                0x10 => println!("Write multiple registers acknowledged"),
                0x0f => println!("Write multiple coils acknowledged"),
                _ => eprintln!("Error: Unexpected function code"), // rarely happen. but different from unsupported fc.
            }
        }
        0x8F | 0x90 => {
            assert_eq!(response_fc, request_fc | 0x80);

            assert_eq!(response_fc, request_fc);

            println!("Modbus exception: {}", response_pdu[1]);
        }
        _ => {
            eprintln!("Unsupported function code")
        }
    }
}
