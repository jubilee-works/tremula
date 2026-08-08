//! Reading back the documents a finished pack run owes, and vouching for them.
//!
//! A pack that reports success has to leave both documents behind; one that is
//! missing is a defect in the pack rather than a verdict, and saying so is the
//! difference between a bug report against tremula and one against the pack.
//!
//! Vouching matters just as much. Results left by an earlier run, or written by
//! a pack that was swapped out between the handshake and the work, would be
//! judged as if they belonged — producing a confident report about something
//! other than what was asked for.

use std::{fs, path::PathBuf};

use tremula_contracts::{
    SCHEMA_VERSION, baseline::Baseline, capabilities::Capabilities, results::Results,
};

use crate::run_dir::RunDir;

/// Why a finished run's documents cannot be judged.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// The pack reported success without leaving a document it owes.
    #[error(
        "the language pack reported success but `{file}` is missing or unreadable ({reason}); this is a defect in the pack — the run directory `{}` is kept for inspection",
        run_dir.display()
    )]
    Missing {
        /// The document that should have been there.
        file: String,
        /// Why it could not be read.
        reason: String,
        /// The run directory, kept as evidence.
        run_dir: PathBuf,
    },
    /// A document in the run directory belongs to something else.
    #[error(
        "the language pack's `{file}` reports {what} `{found}` but this run is `{expected}`; the documents belong to a different run or a different pack — the run directory `{}` is kept for inspection",
        run_dir.display()
    )]
    Foreign {
        /// The document that disagrees.
        file: String,
        /// Which of its claims disagrees.
        what: &'static str,
        /// What it says.
        found: String,
        /// What this run is.
        expected: String,
        /// The run directory, kept as evidence.
        run_dir: PathBuf,
    },
}

/// What a finished pack run left behind.
#[derive(Debug)]
pub struct Artifacts {
    /// The unmutated reference run.
    pub baseline: Baseline,
    /// Every mutant's neutral signals.
    pub results: Results,
}

impl Artifacts {
    /// Read both documents out of `run` and confirm they are this run's, from
    /// the pack the core negotiated with.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError`] when a document is missing, unreadable, or
    /// belongs to another run or another pack.
    pub fn load(run: &RunDir, capabilities: &Capabilities) -> Result<Self, ArtifactError> {
        let baseline: Baseline = read(run, "baseline.json")?;
        let results: Results = read(run, "results.json")?;
        let artifacts = Self { baseline, results };
        artifacts.check_identity(run, capabilities)?;
        Ok(artifacts)
    }

    /// Every claim the documents make about who they belong to.
    fn check_identity(
        &self,
        run: &RunDir,
        capabilities: &Capabilities,
    ) -> Result<(), ArtifactError> {
        let claims: [(&str, &'static str, &str, &str); 6] = [
            (
                "baseline.json",
                "schema version",
                &self.baseline.schema_version,
                SCHEMA_VERSION,
            ),
            (
                "results.json",
                "schema version",
                &self.results.schema_version,
                SCHEMA_VERSION,
            ),
            ("baseline.json", "run", &self.baseline.run_id, run.run_id()),
            ("results.json", "run", &self.results.run_id, run.run_id()),
            (
                "results.json",
                "pack",
                &self.results.pack.name,
                &capabilities.name,
            ),
            (
                "results.json",
                "pack version",
                &self.results.pack.version,
                &capabilities.version,
            ),
        ];
        for (file, what, found, expected) in claims {
            if found != expected {
                return Err(ArtifactError::Foreign {
                    file: file.to_owned(),
                    what,
                    found: found.to_owned(),
                    expected: expected.to_owned(),
                    run_dir: run.path().to_path_buf(),
                });
            }
        }
        Ok(())
    }
}

/// Read one document, reporting its absence as the pack's defect.
fn read<T: serde::de::DeserializeOwned>(run: &RunDir, file: &str) -> Result<T, ArtifactError> {
    let missing = |reason: String| ArtifactError::Missing {
        file: file.to_owned(),
        reason,
        run_dir: run.path().to_path_buf(),
    };
    let document =
        fs::read_to_string(run.path().join(file)).map_err(|err| missing(err.to_string()))?;
    serde_json::from_str(&document).map_err(|err| missing(err.to_string()))
}
