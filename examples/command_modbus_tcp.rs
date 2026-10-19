use std::{process::ExitCode, thread::sleep, time::Duration};

use furious_iot_edge::{
    controlplane::prepare_modbus_tcp,
    db_model::vgateway::VGatewayRow,
    modbus_tcp::{ModbusOperation, ModbusSession},
    scheduler::{ScheduledOp, Scheduler},
};
use rusqlite::{Connection, OpenFlags};

fn main() -> ExitCode {
    let conn =
        Connection::open_with_flags("local-data/example.db", OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let vgateway_record_result = VGatewayRow::find_by_name(&conn, "Local Modbus TCP")
        .unwrap()
        .unwrap();
    let (cmd, skipped) = prepare_modbus_tcp(&conn, &vgateway_record_result.id).unwrap();
    println!("Skipped Operations: {:#?}", skipped);
    let mut scheduler = Scheduler::new();
    cmd.operations.iter().for_each(|op| {
        let operation = ModbusOperation::new(
            op.unit_id,
            op.function_code,
            op.starting_address,
            op.quantity,
        )
        .unwrap();
        let sch_op = ScheduledOp::new(operation, op.poll_interval).unwrap();
        scheduler.add_operation(sch_op);
    });
    let mut session = match ModbusSession::connect(&cmd.address.to_string(), cmd.request_timeout) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Connection error: {e}");
            connect_with_retry(&cmd.address.to_string(), cmd.request_timeout)
        }
    };
    loop {
        if let Err(error) = scheduler.run_all(&mut session) {
            eprintln!("Connection error: {error}");
            session = connect_with_retry(&cmd.address.to_string(), cmd.request_timeout);
        }
    }
}

fn connect_with_retry(address: &str, read_timeout: Duration) -> ModbusSession {
    loop {
        sleep(Duration::from_secs(1));

        match ModbusSession::connect(address, read_timeout) {
            Ok(new_session) => {
                return new_session;
            }
            Err(error) => {
                eprintln!("Reconnect failed: {error}");
            }
        }
    }
}
