use std::{
    io::{Read, Write},
    net::TcpStream,
};

const MBAP_HEADER_LEN: usize = 7;
fn main() {
    let mut client_stream = TcpStream::connect("127.0.0.1:10502").unwrap();

    // PDU

    // FC06 Write single register
    // let request_fc: u8 = 0x06;
    // let request_starting_address: u16 = 0;
    // let request_value: u16 = 0x0000;

    //FC05 Write single coil
    let request_fc: u8 = 0x05;
    let request_starting_address: u16 = 0;
    let request_value: u16 = 0xff00; // ON as 0xff00, OFF as 0x0000

    assert!(matches!(request_fc, 0x05 | 0x06));
    if request_fc == 0x05 {
        assert!(matches!(request_value, 0x0000 | 0xff00));
    }

    let mut request_pdu = [0u8; 5];
    request_pdu[0] = request_fc;
    request_pdu[1..3].copy_from_slice(&request_starting_address.to_be_bytes());
    request_pdu[3..5].copy_from_slice(&request_value.to_be_bytes());

    // MBAP
    let request_tid: u16 = 1;
    let request_pid: u16 = 0;
    let request_uid: u8 = 1;
    let request_mbap_length: u16 = 6; // << Always 6 on modbus tcp read
    let mut request_mbap = [0u8; MBAP_HEADER_LEN];
    request_mbap[0..2].copy_from_slice(&request_tid.to_be_bytes());
    request_mbap[2..4].copy_from_slice(&request_pid.to_be_bytes());
    request_mbap[4..6].copy_from_slice(&request_mbap_length.to_be_bytes());
    request_mbap[6] = request_uid;

    let mut request_adu = [0u8; 12];
    request_adu[..MBAP_HEADER_LEN].copy_from_slice(&request_mbap);
    request_adu[MBAP_HEADER_LEN..].copy_from_slice(&request_pdu);

    client_stream.write_all(&request_adu).unwrap();

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
        // write success route
        0x05 => {
            assert_eq!(response_pdu_len, 5);
            assert_eq!(&response_pdu[..response_pdu_len], &request_pdu);
            println!("write coil acknowledged");
        }
        0x06 => {
            assert_eq!(response_pdu_len, 5);
            assert_eq!(&response_pdu[..response_pdu_len], &request_pdu);
            println!("Write register acknowledged");
        }
        0x81..=0x86 => {
            assert!(response_fc == request_fc || response_fc == (request_fc | 0x80));
            println!("Process Modbus exception");
        }
        _ => {
            eprintln!("Error: Unsupported function code");
        }
    }
}
