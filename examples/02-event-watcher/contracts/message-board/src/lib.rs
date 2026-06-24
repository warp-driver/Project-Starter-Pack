#![no_std]

mod contract;
mod storage;

#[cfg(test)]
mod tests;

// Re-exported so `stellar-handler` can call us via a typed client
// (`MessageBoardClient::new(&env, &addr).record_signed(&id, &msg)`).
// The error enum is part of the published contract spec, so it lives
// in the surface too.
pub use contract::{MessageBoard, MessageBoardClient, MessageBoardError};
