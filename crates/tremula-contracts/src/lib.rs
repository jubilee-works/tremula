//! File contracts shared between the tremula core and its language packs.
//!
//! # Evolution policy
//!
//! Every document carries `schema_version`. Consumers must ignore unknown
//! fields, so adding a field is a compatible change. Adding a variant to an
//! existing enum is **not**: older consumers reject values they do not know,
//! which is intended for `language` (it selects the pack) but must be
//! considered for every other enum.
//!
//! Optional fields always carry `#[serde(default)]` so that an absent key and
//! an explicit `null` mean the same thing. Mirrors in other languages may
//! therefore omit empty values.

pub mod baseline;
pub mod capabilities;
pub mod manifest;
pub mod pack_error;
pub mod report;
pub mod results;
pub mod runner;

/// Version stamped into every contract document produced by this crate. The
/// `contract_version` field in capability and pack metadata reports the same
/// value under a different name.
pub const SCHEMA_VERSION: &str = "0.1";
