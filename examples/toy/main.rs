use crate::{
    modbus_tcp::{
        MultipleWriteRequestDefinition, ReadRequestDefinition, SingleWriteRequestDefinition,
        try_read_once, try_write_multiple_once, try_write_single_once,
    },
    tcp::TcpSession,
};

// The toy exercises only part of the API while the library crate is being prepared.
#[allow(dead_code)]
#[path = "../../src/modbus_tcp/mod.rs"]
mod modbus_tcp;
#[path = "../../src/tcp.rs"]
mod tcp;
fn main() {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:10502".to_owned());
    let mut session = TcpSession::try_connect(&address).unwrap();

    let read1 = ReadRequestDefinition {
        unit_id: 1,
        function_code: 1,
        starting_address: 0,
        quantity: 2000,
    };
    let read2 = ReadRequestDefinition {
        unit_id: 1,
        function_code: 2,
        starting_address: 0,
        quantity: 2000,
    };
    let read3 = ReadRequestDefinition {
        unit_id: 1,
        function_code: 3,
        starting_address: 0,
        quantity: 2,
    };
    let read4 = ReadRequestDefinition {
        unit_id: 2,
        function_code: 4,
        starting_address: 0,
        quantity: 2,
    };
    let write5 = SingleWriteRequestDefinition {
        unit_id: 1,
        starting_address: 0,
        value: modbus_tcp::SingleWriteValue::Coil(true),
    };
    let write6 = SingleWriteRequestDefinition {
        unit_id: 1,
        starting_address: 0,
        value: modbus_tcp::SingleWriteValue::Register(65535),
    };

    let write15 = MultipleWriteRequestDefinition {
        unit_id: 1,
        starting_address: 0,
        values: modbus_tcp::MultipleWriteValues::Coils(vec![
            true, false, true, false, true, false, true, false, true, false, true, false, true,
            false, true, false, true, false, true, false, true, false, true, false, true, false,
            true, false, true, false, true, false, true, false,
        ]),
    };

    let write16 = MultipleWriteRequestDefinition {
        unit_id: 1,
        starting_address: 0,
        values: modbus_tcp::MultipleWriteValues::Registers(vec![
            65535, 0, 65535, 0, 65535, 0, 65535, 0, 65535, 0, 65535, 0, 65535, 0, 65535, 0, 65535,
            0, 65535, 0, 65535, 0, 65535, 0, 65535, 0, 65535, 0,
        ]),
    };

    let read_definitions: Vec<ReadRequestDefinition> = vec![read1, read2, read3, read4];

    for definition in &read_definitions {
        let result = session.execute(|stream| try_read_once(stream, definition, 0));

        match result {
            Ok(data) => {
                println!("Read Result: {:?}", data.bytes_data());
            }
            Err(e) => {
                println!("Read Result: {:?}", e);
            }
        }
    }

    let single_write_definitions: Vec<SingleWriteRequestDefinition> = vec![write5, write6];

    for definition in single_write_definitions {
        let result: Result<modbus_tcp::WriteAck, _> =
            session.execute(|stream| try_write_single_once(stream, &definition, 0));

        match result {
            Ok(ack) => {
                println!("Single write Result: {:?}", ack);
            }
            Err(e) => {
                println!("Single write result: {:?}", e);
            }
        }
    }

    let multiple_write_definition: Vec<MultipleWriteRequestDefinition> = vec![write15, write16];
    for definition in multiple_write_definition {
        let result = session.execute(|stream| try_write_multiple_once(stream, &definition, 0));

        match result {
            Ok(ack) => {
                println!("Multiple write Result: {:?}", ack);
            }
            Err(e) => {
                println!("Multiple write result: {:?}", e);
            }
        }
    }
}
