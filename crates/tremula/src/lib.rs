//! Core logic of tremula: manifest validation, verdict decision rules, and
//! report assembly. Everything here consumes only the shared contracts — it
//! knows nothing about any execution backend.

pub mod artifacts;
pub mod console;
pub mod decision;
pub mod orchestrate;
pub mod pack;
pub mod provenance;
pub mod python_env;
pub mod report;
pub mod run_dir;
pub mod validation;
