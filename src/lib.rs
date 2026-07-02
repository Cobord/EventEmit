//! An event emitter whose registered [`Consumer`](general_emitter::Consumer)s run
//! concurrently by default, but are automatically serialized whenever they don't
//! commute. See the crate [README](https://github.com/) for the full picture:
//! implement [`Interleaves`](interleaving::Interleaves) on your event type to
//! declare which emissions are safe to run at the same time, then drive the
//! rest through [`GeneralEmitter`](general_emitter::GeneralEmitter) /
//! [`SyncEmitter`](general_emitter::SyncEmitter) /
//! [`AsyncEmitter`](general_emitter::AsyncEmitter).

pub mod events;
pub mod general_emitter;
pub mod interleaving;
pub mod tokio_events;
pub mod utils;

pub mod cascading;
