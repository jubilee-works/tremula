//! File contracts shared between the tremula core and its language packs.
//!
//! # Evolution policy
//!
//! Every document carries `schema_version`. Consumers must ignore unknown
//! fields, so adding a field is a compatible change. Adding a variant to an
//! existing enum is compatible on the same terms, but only where the enum has an
//! `Unknown` fallback: `#[serde(other)]` makes an older consumer read a value it
//! has never heard of instead of rejecting the document that carries it. An enum
//! without that fallback — `language`, which selects the pack — breaks older
//! consumers by design, because an unknown language has no pack to run it.
//!
//! A fallback is not a licence to change what a value means. It keeps a document
//! readable; a consumer that meets `Unknown` still knows only that the producer
//! named something newer than itself.
//!
//! An enum with that fallback also has to publish a schema that accepts a value
//! it does not name, or a consumer validating a document before it parses would
//! refuse what its parser was built to read. `open_enum` is what keeps the two
//! halves of the promise together.
//!
//! Optional fields always carry `#[serde(default)]` so that an absent key and
//! an explicit `null` mean the same thing. Mirrors in other languages may
//! therefore omit empty values.

pub mod baseline;
pub mod bundle;
pub mod capabilities;
pub mod manifest;
mod open_enum;
pub mod pack_error;
pub mod probe;
pub mod report;
pub mod results;
pub mod runner;
pub mod spans;
pub mod suppressions;
pub mod triage;

/// Version stamped into every contract document produced by this crate. The
/// `contract_version` field in capability and pack metadata reports the same
/// value under a different name.
pub const SCHEMA_VERSION: &str = "0.1";
