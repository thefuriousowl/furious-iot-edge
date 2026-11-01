use std::{
    io::{Read, Write},
    net::TcpStream,
};

const MBAP_HEADER_LEN: usize = 7;
fn main() {
    let mut client_stream = TcpStream::connect("127.0.0.1:10502").unwrap();

    // // FC03,04
    // let request_fc: u8 = 3;
    // assert!(matches!(request_fc, 0x03 | 0x04));
    // let request_starting_address: u16 = 0;
    // let request_quantity: u16 = 16;
    // assert!((1..=125).contains(&request_quantity));
    // assert!(u32::from(request_starting_address) + u32::from(request_quantity) <= 65536);
    // let mut request_pdu = [0u8; 5];
    // request_pdu[0] = request_fc;
    // request_pdu[1..3].copy_from_slice(&request_starting_address.to_be_bytes());
    // request_pdu[3..5].copy_from_slice(&request_quantity.to_be_bytes());

    //////////////////

    // FC01,02
    let request_fc: u8 = 1;
    assert!(matches!(request_fc, 0x01 | 0x02));
    let request_starting_address: u16 = 0;
    let request_quantity: u16 = 2000; //
    assert!((1..=2000).contains(&request_quantity));
    assert!(u32::from(request_starting_address) + u32::from(request_quantity) <= 65536);
    let mut request_pdu = [0u8; 5];
    request_pdu[0] = request_fc;
    request_pdu[1..3].copy_from_slice(&request_starting_address.to_be_bytes());
    request_pdu[3..5].copy_from_slice(&request_quantity.to_be_bytes());

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
        // success route
        0x01..=0x04 => {
            let response_byte_count = response_pdu[1];

            // validate response function code
            let expected_byte_count = match request_fc {
                0x01 | 0x02 => usize::from(request_quantity.div_ceil(8)),
                0x03 | 0x04 => usize::from(request_quantity) * 2,
                _ => unreachable!(),
            };
            assert_eq!(usize::from(response_byte_count), expected_byte_count);
            assert_eq!(response_pdu_len, 2 + expected_byte_count);

            let data_len = response_pdu_len - 2;

            let response_data = &response_pdu[2..response_pdu_len];
            match response_fc {
                0x01 | 0x02 => {
                    let mut bytes = [0u8; 250];
                    bytes[..data_len].copy_from_slice(response_data);
                    let result = (0..request_quantity as usize)
                        .map(move |i| {
                            let byte_idx = i / 8;
                            let bit_idx = i % 8;
                            ((bytes[byte_idx] >> bit_idx) & 1) == 1
                        })
                        .collect::<Vec<_>>();
                    println!("Byte results: {:?}", result)
                }
                0x03 | 0x04 => {
                    let mut registers = [0u16; 125];
                    let (chunk, _remainder) = response_data.as_chunks::<2>();
                    for (destination, chunk) in
                        registers[..request_quantity as usize].iter_mut().zip(chunk)
                    {
                        *destination = u16::from_be_bytes(*chunk);
                    }
                    println!(
                        "Register results: {:?}",
                        &registers[..request_quantity as usize]
                    )
                }
                _ => {
                    eprintln!("Error: Unsupported function code");
                }
            }
        }
        // exception route
        0x81..=0x84 => {
            assert_eq!(response_pdu_len, 2);
            println!("Process Modbus exception")
        }
        _ => {
            eprintln!("Error: Unsupported function code");
        }
    }
}
