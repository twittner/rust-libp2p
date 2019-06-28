mod error;
mod manager;
mod task;

pub use error::Error;
pub use manager::{TaskEntryRef, Manager, Event};
pub use task::ClosedTask;

/// Identifier for a future that attempts to reach a node.
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(usize);

