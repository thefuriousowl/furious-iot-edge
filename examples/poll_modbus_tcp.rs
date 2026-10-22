use std::{thread::sleep, time::Duration};

use furious_iot_edge::{
    modbus_tcp::{
        FunctionCode::{self, ReadInputRegisters},
        ModbusError, ModbusOperation, ModbusSession,
    },
    scheduler::{ScheduledOp, Scheduler},
    shared_state::SnapshotStore,
};

const TEMP_READ_TIMEOUT: Duration = Duration::from_millis(3000);

fn main() -> Result<(), ModbusError> {
    let mut session = ModbusSession::connect("127.0.0.1:10502", TEMP_READ_TIMEOUT)?;
    let operation_fc03 =
        ModbusOperation::new(0x01, FunctionCode::ReadHoldingRegisters, 0x0000, 0x0002)
            .expect("should be valid");
    let operation_fc04 =
        ModbusOperation::new(0x02, FunctionCode::ReadInputRegisters, 0x0000, 0x0002)
            .expect("should be valid");

    let another_fc04_operation =
        ModbusOperation::new(0x02, FunctionCode::ReadInputRegisters, 0x0000, 0x0010)
            .expect("should be valid");

    let exception_operation =
        ModbusOperation::new(0x02, ReadInputRegisters, 100, 2).expect("should be valid");

    let scheduled_operation_fc03 =
        ScheduledOp::new("op-1", operation_fc03, Duration::from_millis(1000))
            .expect("valid scheduled_op");
    let scheduled_operation_fc04 =
        ScheduledOp::new("op-2", operation_fc04, Duration::from_millis(5000))
            .expect("valid scheduled_op");
    let scheduled_another_fc04_operation =
        ScheduledOp::new("op-3", another_fc04_operation, Duration::from_millis(3000))
            .expect("valid scheduled_op");

    let scheduled_exception_operation =
        ScheduledOp::new("op-4", exception_operation, Duration::from_millis(2000))
            .expect("valid scheduled_op");

    let mut scheduler = Scheduler::new();
    let store = SnapshotStore::new();
    scheduler.add_operation(scheduled_operation_fc03);
    scheduler.add_operation(scheduled_operation_fc04);
    scheduler.add_operation(scheduled_another_fc04_operation);
    scheduler.add_operation(scheduled_exception_operation);
    loop {
        match scheduler.run_all(&mut session, &store) {
            Err(error) => {
                eprintln!("Connection error: {error}");
                session = connect_with_retry("127.0.0.1:10502", TEMP_READ_TIMEOUT);
            }
            Ok(_) => return Ok(()),
        }
    }
}

fn connect_with_retry(address: &str, read_timeout: Duration) -> ModbusSession {
    loop {
        sleep(Duration::from_secs(1));

        match ModbusSession::connect(address, read_timeout) {
            Ok(new_session) => return new_session,
            Err(error) => {
                eprintln!("Reconnect failed: {error}");
            }
        }
    }
}
