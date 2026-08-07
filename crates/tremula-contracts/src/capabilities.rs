//! What a language pack tells the core about itself before any work starts.
//! The core refuses to continue when the pack cannot serve the contract
//! version or the subcommands it needs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A pack's self-description, printed by its `--capabilities` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    /// Distribution name, for example `tremula-python`.
    pub name: String,
    /// Pack version.
    pub version: String,
    /// Contract version the pack implements, the same value documents carry as
    /// `schema_version`.
    pub contract_version: String,
    /// Subcommands the pack accepts.
    pub subcommands: Vec<String>,
    /// Named language-level checks the pack's `validate` performs, so a newer
    /// core can tell whether the checks it wants are available.
    pub validate_checks: Vec<String>,
}
