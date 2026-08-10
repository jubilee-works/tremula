//! Core logic of tremula: asking a model for mutants, manifest validation,
//! verdict decision rules, and report assembly. Everything here consumes only
//! the shared contracts — it knows nothing about any execution backend.

pub mod artifacts;
pub mod child;
pub mod console;
pub mod decision;
pub mod generate;
pub mod orchestrate;
pub mod pack;
pub mod provenance;
pub mod python_env;
pub mod report;
pub mod run_dir;
pub mod validation;
