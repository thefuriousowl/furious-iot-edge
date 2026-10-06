use std::{thread::sleep, time::Duration};

use crate::{
    modbus_tcp::{FunctionCode, ModbusError, ModbusOperation, ModbusSession},
    scheduler::{ScheduledOp, Scheduler},
};

mod modbus_tcp;
mod scheduler;

fn main() -> Result<(), ModbusError> {
    let mut session = ModbusSession::connect("127.0.0.1:10502")?;
    let operation_fc03 =
        ModbusOperation::new(0x01, FunctionCode::ReadHoldingRegisters, 0x0000, 0x0002)
            .expect("should be valid");
    let operation_fc04 =
        ModbusOperation::new(0x02, FunctionCode::ReadInputRegisters, 0x0000, 0x0002)
            .expect("should be valid");

    let another_fc04_operation =
        ModbusOperation::new(0x02, FunctionCode::ReadInputRegisters, 0x0000, 0x0010)
            .expect("should be valid");

    let scheduled_operation_fc03 =
        ScheduledOp::new(operation_fc03, Duration::from_millis(1000)).expect("valid scheduled_op");
    let scheduled_operation_fc04 =
        ScheduledOp::new(operation_fc04, Duration::from_millis(5000)).expect("valid scheduled_op");
    let scheduled_another_fc04_operation =
        ScheduledOp::new(another_fc04_operation, Duration::from_millis(3000))
            .expect("valid scheduled_op");

    let mut scheduler = Scheduler::new();
    scheduler.add_operation(scheduled_operation_fc03);
    scheduler.add_operation(scheduled_operation_fc04);
    scheduler.add_operation(scheduled_another_fc04_operation);
    loop {
        match scheduler.run_all(&mut session) {
            Err(ModbusError::Io(error)) => {
                eprintln!("Connection error: {error}");

                loop {
                    sleep(Duration::from_secs(1));

                    match ModbusSession::connect("127.0.0.1:10502") {
                        Ok(new_session) => {
                            session = new_session;
                            break;
                        }
                        Err(error) => {
                            eprintln!("Reconnect failed: {error}");
                        }
                    }
                }
            }
            Err(error) => return Err(error),
            Ok(_) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
