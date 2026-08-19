//! Choosing what to mutate out of what a change touched and what a test suite reached.
//!
//! Two inputs, each answering a question the other cannot. A diff says which lines this
//! change is responsible for, without which every run would be about the whole project
//! again. Coverage says which of those lines a test ever reached, without which a
//! mutation is planted where no suite could have caught it and its survival says
//! nothing about anybody's tests.
//!
//! Nothing here asks a model anything. Every step is arithmetic over documents, so the
//! same change and the same coverage select the same functions every time.

pub mod diff;
pub mod lcov;
pub mod lines;
