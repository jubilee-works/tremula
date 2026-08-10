//! What the core requires of a pack before it will use it.
//!
//! Negotiated once, at the start of whatever command needs the pack, and never
//! discovered half way through: a contract version that does not match, a subcommand
//! the pack does not offer, or a pack with no checks of its own to run over a
//! manifest are all reasons to refuse before a model has been paid for anything.

use tremula_contracts::{SCHEMA_VERSION, capabilities::Capabilities};

use crate::{
    pack::{
        failures::PackError,
        session::{drive, neutral, pack_command, unwatched},
    },
    python_env::PythonEnv,
};

/// The subcommands a pack has to offer before the core will use it.
pub const REQUIRED_SUBCOMMANDS: [&str; 3] = ["run", "collect", "validate"];

/// The one a generation needs on top of those.
///
/// Only a generation asks for it, and only a core that asks for it needs it, which
/// is why it is not among the subcommands every run requires: a pack that predates
/// this call still runs a manifest somebody else generated.
pub const SPANS_SUBCOMMAND: &str = "spans";

/// The one triage needs, on top of `spans`.
///
/// Held to the same rule for the same reason: a pack that cannot run a witness can
/// still run a manifest, and refusing it a run over a call it will never be asked
/// to make would be refusing it for nothing.
pub const PROBE_SUBCOMMAND: &str = "probe";

/// Negotiate with the pack installed in `env`.
///
/// The core refuses a pack it cannot rely on rather than discovering the
/// mismatch half way through a run: the contract version has to match exactly,
/// every subcommand a run uses has to be offered, and a pack that performs no
/// language checks is not a pack this core can validate a manifest with.
///
/// # Errors
///
/// Returns [`PackError`] when the pack cannot be run, does not describe itself,
/// or describes itself as something the core cannot use.
pub fn handshake(env: &PythonEnv) -> Result<Capabilities, PackError> {
    negotiate(env, &[])
}

/// Negotiate with the pack, requiring what a generation needs as well.
///
/// # Errors
///
/// As [`handshake`], and additionally when the pack cannot say where a mutation
/// may land in a file — which a generation has no way to find out for itself.
pub fn handshake_for_generation(env: &PythonEnv) -> Result<Capabilities, PackError> {
    negotiate(env, &[SPANS_SUBCOMMAND])
}

/// Negotiate with the pack, requiring what triage needs as well.
///
/// Both of the extra calls are checked here rather than at the point of use, and
/// before a model is paid for anything: triage asks the pack where a function is
/// and then asks it to run one input against that function, and finding out at the
/// second of those that the pack cannot would have spent a call per survivor to
/// learn it.
///
/// # Errors
///
/// As [`handshake`], and additionally when the pack cannot say where a function is
/// or cannot run one input against two versions of it.
pub fn handshake_for_triage(env: &PythonEnv) -> Result<Capabilities, PackError> {
    negotiate(env, &[SPANS_SUBCOMMAND, PROBE_SUBCOMMAND])
}

fn negotiate(env: &PythonEnv, also: &[&str]) -> Result<Capabilities, PackError> {
    let mut command = pack_command(env);
    command.arg("--capabilities").current_dir(neutral());
    let transcript = drive(env, command, None, false, &unwatched)?;
    if !transcript.succeeded() {
        return Err(transcript.into_failure(None));
    }
    let document = transcript.last_line().unwrap_or_default();
    let capabilities: Capabilities =
        serde_json::from_str(&document).map_err(|err| PackError::UnreadableHandshake {
            reason: err.to_string(),
        })?;
    check(&capabilities, also)?;
    Ok(capabilities)
}

/// Everything the core requires of a pack it has just met.
fn check(capabilities: &Capabilities, also: &[&str]) -> Result<(), PackError> {
    if capabilities.contract_version != SCHEMA_VERSION {
        return Err(PackError::ContractMismatch {
            pack: capabilities.contract_version.clone(),
        });
    }
    let missing: Vec<String> = REQUIRED_SUBCOMMANDS
        .iter()
        .chain(also)
        .filter(|required| !capabilities.subcommands.iter().any(|had| had == *required))
        .map(|required| (*required).to_owned())
        .collect();
    if !missing.is_empty() {
        return Err(PackError::MissingSubcommands { missing });
    }
    if capabilities.validate_checks.is_empty() {
        return Err(PackError::NoLanguageChecks {
            pack: capabilities.name.clone(),
        });
    }
    Ok(())
}
