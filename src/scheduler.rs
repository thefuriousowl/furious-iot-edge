use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use crate::{
    modbus_tcp::{ModbusError, ModbusOperation, ModbusResponse, ModbusSession},
    shared_state::{RegisterSnapshot, SnapshotStore},
};

pub const MIN_ALLOW_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct ScheduledOp {
    operation_id: String,
    operation: ModbusOperation,
    interval: Duration,
    next_due: Instant,
}
impl ScheduledOp {
    pub fn new(id: &str, operation: ModbusOperation, interval: Duration) -> Result<Self, String> {
        if interval < MIN_ALLOW_INTERVAL {
            return Err(format!("interval must be at least {MIN_ALLOW_INTERVAL:?}"));
        }
        Ok(Self {
            operation_id: id.to_string(),
            operation,
            interval,
            next_due: Instant::now(),
        })
    }

    pub(crate) fn is_due(&self) -> bool {
        Instant::now() >= self.next_due
    }
    pub(crate) fn time_until_due(&self) -> Duration {
        self.next_due.saturating_duration_since(Instant::now())
    }
    pub(crate) fn set_next_due(&mut self, now: Instant) {
        self.next_due += self.interval;

        while self.next_due <= now {
            self.next_due += self.interval
        }
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

#[derive(Debug, Default)]
pub struct Scheduler {
    ops: Vec<ScheduledOp>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { ops: vec![] }
    }

    pub fn add_operation(&mut self, scheduled_op: ScheduledOp) {
        self.ops.push(scheduled_op);
    }

    pub fn run_all(
        &mut self,
        session: &mut ModbusSession,
        store: &SnapshotStore,
    ) -> Result<(), ModbusError> {
        loop {
            let min_wait = self
                .ops
                .iter()
                .map(|op| op.time_until_due())
                .min()
                .unwrap_or(Duration::ZERO);

            if !min_wait.is_zero() {
                sleep(min_wait);
            }

            for op in &mut self.ops {
                if op.is_due() {
                    let result = session.execute(&op.operation);
                    let now = Instant::now();
                    match result {
                        Ok(ModbusResponse::Registers { data, count }) => {
                            store.put(op.operation_id(), RegisterSnapshot::from(data, count, now));
                        }
                        Ok(ModbusResponse::Exception(exception)) => {
                            println!("T: {:?}, exception: {:?}", now, exception)
                        }
                        Err(error) if error.connection_still_usable() => {
                            eprintln!("Connection error: {error}");
                        }

                        Err(error) => return Err(error),
                    }

                    op.set_next_due(now);
                }
            }
        }
    }

    pub fn ops(&self) -> &[ScheduledOp] {
        &self.ops
    }
}
