use crate::group::Group;
use dashmap::DashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct HttpServerState {
    // TODO: Just an experiment of independent state to the groups
    pub counter: std::sync::atomic::AtomicU64,
    pub groups: Arc<DashMap<String, Group>>,
}

impl HttpServerState {
    pub fn new(groups: Arc<DashMap<String, Group>>) -> Self {
        Self {
            counter: std::sync::atomic::AtomicU64::default(),
            groups,
        }
    }
}
