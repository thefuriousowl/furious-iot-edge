use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

pub struct SnapshotStore {
    latest: Mutex<HashMap<String, RegisterSnapshot>>,
}
impl SnapshotStore {
    pub fn new() -> Self {
        Self {
            latest: Mutex::new(HashMap::new()),
        }
    }
    pub fn put(&self, operation_id: &str, snapshot: RegisterSnapshot) {
        let mut guard = self.latest.lock().unwrap();
        guard.insert(operation_id.to_string(), snapshot);
    }
    pub fn get(&self, operation_id: &str) -> Option<RegisterSnapshot> {
        self.latest.lock().unwrap().get(operation_id).copied()
    }
}
impl Default for SnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RegisterSnapshot {
    data: [u16; 125],
    count: usize,
    acquired_at: Instant,
}

impl RegisterSnapshot {
    pub fn from(data: [u16; 125], count: usize, acquired_at: Instant) -> Self {
        Self {
            data,
            count,
            acquired_at,
        }
    }

    pub fn registers(&self) -> &[u16] {
        &self.data[..self.count]
    }

    pub fn age(&self) -> Duration {
        self.acquired_at.elapsed()
    }

    pub fn acquired_at(&self) -> Instant {
        self.acquired_at
    }
}
