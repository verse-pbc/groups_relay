use crate::Groups;
use std::sync::Arc;

#[derive(Debug)]
pub struct HttpServerState {
    // TODO: Just an experiment of independent state to the groups
    pub counter: std::sync::atomic::AtomicU64,
    pub groups: Arc<Groups>,
}

impl HttpServerState {
    pub fn new(groups: Arc<Groups>) -> Self {
        Self {
            counter: std::sync::atomic::AtomicU64::default(),
            groups,
        }
    }
}
