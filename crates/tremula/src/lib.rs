//! Core logic of tremula: manifest validation, verdict decision rules, and
//! report assembly. Everything here consumes only the shared contracts — it
//! knows nothing about any execution backend.

pub mod console;
pub mod decision;
pub mod report;
pub mod validation;
