use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use crate::modbus_tcp::{ModbusError, ModbusOperation, ModbusResponse, ModbusSession};

pub const MIN_ALLOW_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub(crate) struct RegisterSnapshot {
    data: [u16; 125],
    count: usize,
    acquired_at: Instant,
}

impl RegisterSnapshot {
    pub(crate) fn from(data: [u16; 125], count: usize, acquired_at: Instant) -> Self {
        Self {
            data,
            count,
            acquired_at,
        }
    }
}

pub struct ScheduledOp {
    operation: ModbusOperation,
    interval: Duration,
    next_due: Instant,
    last_success: Option<RegisterSnapshot>,
}
impl ScheduledOp {
    pub fn new(operation: ModbusOperation, interval: Duration) -> Result<Self, String> {
        if interval < MIN_ALLOW_INTERVAL {
            return Err(format!("interval must be at least {MIN_ALLOW_INTERVAL:?}"));
        }
        Ok(Self {
            operation,
            interval,
            next_due: Instant::now(),
            last_success: None,
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

    pub(crate) fn store_registers_snapshot(
        &mut self,
        data: [u16; 125],
        count: usize,
        now: Instant,
    ) {
        self.last_success = Some(RegisterSnapshot::from(data, count, now))
    }

    pub(crate) fn last_success(&self) -> Option<(&[u16], Instant)> {
        match &self.last_success {
            None => None,
            Some(snapshot) => Some((&snapshot.data[..snapshot.count], snapshot.acquired_at)),
        }
    }
}

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

    pub fn run_all(&mut self, session: &mut ModbusSession) -> Result<ModbusResponse, ModbusError> {
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
                    println!("Previous: {:?}", op.last_success());
                    let result = session.execute(&op.operation);
                    let now = Instant::now();
                    match result {
                        Ok(ModbusResponse::Registers { data, count }) => {
                            op.store_registers_snapshot(data, count, now);

                            println!(
                                "Last Success: {:?}, registers: {:?}",
                                now,
                                op.last_success()
                            )
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

    pub(crate) fn ops(&self) -> &[ScheduledOp] {
        &self.ops
    }
}
