#![no_std]
extern crate alloc;

mod contract;
mod storage;

#[cfg(test)]
mod tests;

pub use contract::{RecordPayload, StellarHandler, StellarHandlerClient};
// Re-export the shared protocol types so callers (tests, frontends,
// integration crates) can `use stellar_handler::*` and get one
// consistent surface instead of having to depend on `warpdrive-shared`
// directly. Mirrors hodlers-app/contracts/stellar-handler.
pub use warpdrive_shared::interfaces::handler::{Ed25519SignatureData, HandlerError};
