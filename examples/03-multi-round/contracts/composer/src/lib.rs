#![no_std]
extern crate alloc;

mod contract;
mod storage;

#[cfg(test)]
mod tests;

pub use contract::{
    Composer, ComposerClient, ComposerError, FinalPayload, Finalized, Round1Attestation,
    Round1Bundle, Round1Payload, Round1Ready, SubmissionPayload,
};
// Re-export the shared protocol types so callers (tests, frontends,
// integration crates) can `use composer::*` and get one consistent
// surface instead of having to depend on `warpdrive-shared` directly.
pub use warpdrive_shared::interfaces::handler::Ed25519SignatureData;
