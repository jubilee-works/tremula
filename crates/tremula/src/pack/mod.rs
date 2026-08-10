//! Driving a language pack over the protocol it publishes.
//!
//! Four rules hold for every call. The pack is run as an installed module
//! (`-m`) through an isolated interpreter (`-I`), so that neither the working
//! directory nor the environment can put a different `tremula_python` in front
//! of the one the core negotiated with. The paths the pack resolves for itself —
//! the manifest, the project root, the run directory — are handed over absolute,
//! because the pack starts subprocesses of its own with working directories of
//! their own; what the caller wrote after `--tests` is passed through exactly as
//! given, since it is the project's own test runner that interprets it. The
//! pack's own account of a failure — its last line of stdout — is the only
//! failure report the core reads, because an execution backend may discard
//! stderr. And no call leaves a pack process behind, however it ends.
//!
//! Those rules are kept in [`session`], which every call goes through. What the
//! calls are is in [`calls`], what the core requires of a pack before making any of
//! them is in [`handshake`], and what it makes of a call that failed is in
//! [`failures`]. This module is their table of contents: the whole of the pack's
//! surface is named here, so that a caller reads `pack::probe` and `pack::PackError`
//! and never has to know which file either lives in.

mod calls;
pub mod failures;
mod handshake;
mod session;

pub use calls::{ProbeRequest, RunRequest, probe, run, spans, validate_deep};
pub use failures::PackError;
pub use handshake::{
    PROBE_SUBCOMMAND, REQUIRED_SUBCOMMANDS, SPANS_SUBCOMMAND, handshake, handshake_for_generation,
    handshake_for_triage,
};
