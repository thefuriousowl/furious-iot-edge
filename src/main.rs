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
        let quantity = 16u16;
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
