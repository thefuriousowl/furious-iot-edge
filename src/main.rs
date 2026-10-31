use std::{
    io::{Read, Write}, net::{SocketAddr, TcpStream}, time::Duration,
};

struct ModbusRequest {
    function_code: u8,
    starting_address: u16,
    quantity: u16,
    unit_id: u8,
}



struct ModbusTcpReader {
    tcp_stream: TcpStream,
    transaction_id: u16,
}
impl ModbusTcpReader {
    fn try_connect(socket_address: &str, read_timeout: Duration) -> Result<Self, std::io::Error> {
        let parsed_address = socket_address.parse::<SocketAddr>().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let tcp_stream = TcpStream::connect(parsed_address)?;

        tcp_stream.set_read_timeout(Some(read_timeout))?;
        Ok(Self {
            tcp_stream,
             transaction_id: 0,
        })
    }

    fn transaction_id(&self) -> u16 {
        self.transaction_id
    }

    // return current request transaction id
    // then increase self transaction id by 1
    fn process_transaction_id(&mut self) -> u16 {
        let current_transaction_id = self.transaction_id;
        self.transaction_id = self.transaction_id.wrapping_add(1);
        current_transaction_id
    }


}

const MBAP_HEADER_LEN: usize = 7;
fn main() {
    let mut modbus_tcp_reader = ModbusTcpReader::try_connect("127.0.0.1:10502", Duration::from_millis(3000)).unwrap();

    let request = ModbusRequest { function_code: 3, starting_address: 0, quantity: 2, unit_id: 1};
    // PDU
    let mut request_pdu = [0u8; 5];
    request_pdu[0] = request.function_code;
    request_pdu[1..3].copy_from_slice(&request.starting_address.to_be_bytes());
    request_pdu[3..5].copy_from_slice(&request.quantity.to_be_bytes());

    // MBAP
    // let request_tid: u16 = 1;
    let request_pid: u16 = 0;
    // let request_uid: u8 = 1;

    let request_mbap_length: u16 = 6; // << Always 6 on modbus tcp read
    let mut request_mbap = [0u8; MBAP_HEADER_LEN];
    request_mbap[0..2].copy_from_slice(&modbus_tcp_reader.transaction_id().to_be_bytes());
    request_mbap[2..4].copy_from_slice(&request_pid.to_be_bytes());
    request_mbap[4..6].copy_from_slice(&request_mbap_length.to_be_bytes());
    request_mbap[6] = request.unit_id;

    let mut request_adu = [0u8; 12];
    request_adu[..MBAP_HEADER_LEN].copy_from_slice(&request_mbap);
    request_adu[MBAP_HEADER_LEN..].copy_from_slice(&request_pdu);

    modbus_tcp_reader.tcp_stream.write_all(&request_adu).unwrap();
    let request_tid = modbus_tcp_reader.process_transaction_id();

    // Modbus Response
    let mut response_mbap = [0u8; MBAP_HEADER_LEN];
    modbus_tcp_reader.tcp_stream.read_exact(&mut response_mbap).unwrap();

    let response_length = u16::from_be_bytes([response_mbap[4], response_mbap[5]]);
    if !(2..254).contains(&response_length) {
        eprintln!("Error: Invalid response MBAP Length");
    }

    let response_pdu_len = usize::from(response_length - 1);
    if response_pdu_len < 2 {
        if response_pdu_len == 1 {
            let mut _thrash_byte = [0u8; 1];
            modbus_tcp_reader.tcp_stream.read_exact(&mut _thrash_byte).unwrap();
        }
        eprintln!("Error: PDU truncated");
    }

    let mut response_pdu = [0u8; 253];

    modbus_tcp_reader.tcp_stream
        .read_exact(&mut response_pdu[..response_pdu_len])
        .unwrap();

    // Read and validate response transaction ID
    let response_tid = u16::from_be_bytes([response_mbap[0], response_mbap[1]]);
    if response_tid != request_tid{
        eprintln!("Error: TransactionIdMismatch");
    }

    // Read and validate response protocol ID
    let response_pid = u16::from_be_bytes([response_mbap[2], response_mbap[3]]);
    if response_pid != request_pid {
        eprintln!("Error: InvalidProtocolId");
    }

    // Read and validate unit ID
    let response_uid = response_mbap[6];
    if response_uid != request.unit_id {
        eprintln!("Error: UnitIdMismatch");
    }

    // Read and validate function code
    let response_fc = response_pdu[0];
    if response_fc != request.function_code && response_fc != (request.function_code | 0x80) {
        eprintln!("Error: FunctionCodeMismatch")
    }

    // success or exception
    match response_fc {
        // success route
        0x01..=0x04 => {
            if response_pdu.len() < 2 {
                eprintln!("Error: PduTruncated");
            }
            let response_byte_count = response_pdu[1];

            // validate response function code
            match response_fc {
                0x01..=0x02 => {
                    if response_byte_count as u16 != request.quantity.div_ceil(8) {
                        eprintln!("Error: ByteCountMismatched");
                    }
                }
                0x03..=0x04 => {
                    if response_byte_count as u16 != request.quantity * 2 {
                        eprintln!("Error: ByteCountMismatched");
                    }
                }
                _ => {
                    eprintln!("Error: Unsupported function code");
                }
            }

            let data_len = response_pdu_len - 2;
            if response_byte_count as usize != data_len {
                eprintln!("Error: ActualDataLenMismatchByteCount")
            }

            let response_data = &response_pdu[2..response_pdu_len];
            match response_fc {
                0x01 | 0x02 => {
                    let mut bytes = [0u8; 250];
                    bytes[..data_len].copy_from_slice(response_data);
                    let result = (0..request.quantity as usize)
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
                        registers[..request.quantity as usize].iter_mut().zip(chunk)
                    {
                        *destination = u16::from_be_bytes(*chunk);
                    }
                    println!(
                        "Register results: {:?}",
                        &registers[..request.quantity as usize]
                    )
                }
                _ => {
                    eprintln!("Error: Unsupported function code");
                }
            }
        }
        0x81..=0x84 => {
            println!("Process Modbus exception")
        }
        _ => {
            eprintln!("Error: Unsupported function code");
        }
    }
}
