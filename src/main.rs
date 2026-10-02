use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    thread::sleep,
    time::Duration,
};

const MB_TCP_PID: u16 = 0;
const MBAP_HEADER_LEN: usize = 7;




#[derive(Debug)]
pub(crate) struct ModbusSession {
    stream: TcpStream,
    next_transaction_id: u16,
}

impl ModbusSession {
    pub(crate) fn connect(socket_address: &str) -> Result<Self, std::io::Error> {
        let parsed_address = socket_address
            .parse::<SocketAddr>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let stream = TcpStream::connect(parsed_address)?;
        Ok(Self {
            stream,
            next_transaction_id: 0,
        })
    }

    pub(crate) fn increment_transaction_id(&mut self) {
        self.next_transaction_id = self.next_transaction_id.wrapping_add(1);
    }

    pub(crate) fn execute(&mut self) -> std::io::Result<()> {
        let starting_address = 0u16;
        let quantity = 2u16;
        let fc = 3u8;
        let mut req_pdu = [0u8; 5];
        req_pdu[0] = fc;
        req_pdu[1..3].copy_from_slice(&starting_address.to_be_bytes());
        req_pdu[3..5].copy_from_slice(&quantity.to_be_bytes());

        let mut req_mbap = [0u8; 7];
        let req_mbap_length: u16 = 6;
        let req_uid = 1;
        let req_tid = self.next_transaction_id;
        req_mbap[0..2].copy_from_slice(&req_tid.to_be_bytes());
        req_mbap[2..4].copy_from_slice(&MB_TCP_PID.to_be_bytes());
        req_mbap[4..6].copy_from_slice(&req_mbap_length.to_be_bytes());
        req_mbap[6] = req_uid;

        let mut adu = [0u8; 12];
        adu[..7].copy_from_slice(&req_mbap);
        adu[7..].copy_from_slice(&req_pdu);
        self.increment_transaction_id();
        self.stream.write_all(&adu)?;
        let mut res_mbap = [0u8; MBAP_HEADER_LEN];

        self.stream.read_exact(&mut res_mbap)?;

        let res_length = u16::from_be_bytes([res_mbap[4], res_mbap[5]]);
        if !(2..=254).contains(&res_length) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid MBAP length field",
            ));
        }

        let pdu_len = usize::from(res_length - 1);
        if pdu_len < 2 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PDU Truncated",
            ));
        }

        let mut res_pdu = [0u8; 253];
        // Read FC
        self.stream.read_exact(&mut res_pdu[..pdu_len])?;
        let res_fc = res_pdu[0];

        let res_tid = u16::from_be_bytes([res_mbap[0], res_mbap[1]]);
        if res_tid != req_tid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response transaction id mismatch from request transaction id",
            ));
        }
        let res_pid = u16::from_be_bytes([res_mbap[2], res_mbap[3]]);
        if res_pid != MB_TCP_PID {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid response protocol id",
            ));
        }

        let res_uid = res_mbap[6];
        if req_uid != res_uid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response unit id mismatch from request unit id",
            ));
        }

        match res_fc {
            0x03 => {
                let res_byte_count = res_pdu[1];
                if (res_byte_count as u16) != quantity * 2 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Byte count mismatch",
                    ));
                }
                let data_len = pdu_len - 2;
                if (res_byte_count as usize) != data_len {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Byte count mismatch",
                    ));
                }

                let res_data = &res_pdu[2..pdu_len];
                let mut raw_registers = [0u16; 125];
                let (chunks, _remainder) = res_data.as_chunks::<2>();
                for (dst, chunk) in raw_registers[..quantity.into()].iter_mut().zip(chunks) {
                    *dst = u16::from_be_bytes(*chunk);
                }
                let registers = &raw_registers[..quantity as usize];
                println!("TID: {}, Registers: {:?}", res_tid, registers);
            }
            0x83 => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Exception Response (will be implement later)",
                ));
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unknown FC",
                ));
            }
        }

        Ok(())
    }
}

fn main() -> std::io::Result<()> {
    let mut session = ModbusSession::connect("127.0.0.1:10502").expect("good");
    loop {
        session.execute()?;
        sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    use crate::ModbusSession;
    #[test]
    fn valid_fc03_round_trip() {
        let server = TcpListener::bind("127.0.0.1:0").expect("should bind test server");
        let server_addr = server.local_addr().expect("should get local address");
        // 12-byte Modbus TCP Request ADU (FC03 Read Holding Registers)
        // Transaction ID (2B), Protocol ID (2B), Length (2B), Unit ID (1B), FC (1B), Start Addr (2B), Quantity (2B)
        let expected_request_adu: [u8; 12] = [
            0x00, 0x00, // Transaction ID: 0
            0x00, 0x00, // Protocol ID: 0 (Modbus TCP)
            0x00, 0x06, // Length: 6 bytes following
            0x01, // Unit ID: 1
            0x03, // Function Code: 03 (Read Holding Registers)
            0x00, 0x00, // Starting Address: 0
            0x00, 0x02, // Quantity of Registers: 2
        ];

        // Modbus TCP Response ADU (2 registers = 4 bytes response data)
        // Header (7B) + FC (1B) + Byte Count (1B) + Register Data (4B) = 13 bytes
        let response_adu: [u8; 13] = [
            0x00, 0x00, // Transaction ID: 0 (Must be matched the Request)
            0x00, 0x00, // Protocol ID: 0
            0x00, 0x07, // Length: 7 bytes following (1 Unit ID + 1 FC + 1 ByteCount + 4 Data)
            0x01, // Unit ID: 1
            0x03, // Function Code: 03
            0x04, // Byte Count: 4 bytes (2 registers * 2 bytes)
            0x00, 0x0a, // Register 0 value: 10
            0x00, 0x14, // Register 1 value: 20
        ];
        let server_thread = std::thread::spawn(move || {
            let (mut server_stream, _) = server.accept().expect("should be ok");
            // Read exact 12-byte request
            let mut buf = [0u8; 12];
            server_stream
                .read_exact(&mut buf)
                .expect("server should read 12-byte request");

            // Assert request bytes
            assert_eq!(
                buf, expected_request_adu,
                "Request ADU does not match expected bytes"
            );

            // Write valid FC03 response
            server_stream
                .write_all(&response_adu)
                .expect("server should write response");
            server_stream.flush().expect("should flush server_stream");
        });

        let mut session =
            ModbusSession::connect(&server_addr.to_string()).expect("client should connect");
        session.execute().expect("execute should succeed");

        server_thread
            .join()
            .expect("server thread panicked or failed");
    }
}
