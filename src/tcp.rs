use std::{io, net::TcpStream};

pub struct TcpSession {
    stream: TcpStream,
}
impl TcpSession {
    pub fn try_connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self { stream })
    }
    pub fn execute<R>(&mut self, operation: impl FnOnce(&mut TcpStream) -> R) -> R {
        operation(&mut self.stream)
    }
}
