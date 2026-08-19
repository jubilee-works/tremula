//! Core logic of tremula: asking a model for mutants, manifest validation,
//! verdict decision rules, and report assembly. Everything here consumes only
//! the shared contracts — it knows nothing about any execution backend.

pub mod artifacts;
pub mod bundle;
pub mod child;
pub mod comment;
pub mod console;
pub mod decision;
pub mod dismiss;
pub mod generate;
pub mod orchestrate;
pub mod pack;
mod paths;
pub mod provenance;
pub mod python_env;
pub mod report;
pub mod run_dir;
pub mod suppressions;
pub mod triage;
pub mod validation;
