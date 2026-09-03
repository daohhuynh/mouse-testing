//! Sensor behaviour: what the optical sensor and the firmware between it and the
//! wire actually do to motion.
//!
//! Every detector here is a pure function over a slice of [`types::Report`]
//! returning a plain-old-data struct. None of them allocate on a capture
//! thread, do I/O, or hold state, so the capture path is never waiting on
//! analysis and the UI can call any of them mid-frame.
//!
//! Each one also has a protocol it needs the user to follow, and each will
//! return [`Verdict::Inconclusive`] rather than a number when the protocol was
//! not met. That ordering is deliberate: a refusal to answer is a better
//! result than an answer computed from a stroke that was too slow, too short,
//! or too fast to carry the information.

pub mod cpi;
pub mod drift;
pub mod protocol;
pub mod seg;
pub mod smooth;
pub mod snap;
pub mod tracking;
pub mod types;
pub mod util;

#[cfg(test)]
pub mod mousesim;
#[cfg(test)]
mod tests;

pub use types::{Report, Verdict};
