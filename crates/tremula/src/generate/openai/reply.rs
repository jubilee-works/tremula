//! What an answer about mutations amounts to, once its envelope has been read.
//!
//! The five things an answer can be are decided one level down, by code that knows
//! nothing about mutations. This is where they become the vocabulary a generation
//! reports in: nothing another ask would fix, or a defect the model can be told
//! about and asked about again. The count belongs here for the same reason — a
//! schema cannot say "four", so a well-formed answer with three mutations in it is
//! read as the contract and then refused by it.

use super::{MAX_COMPLETION_TOKENS, answer::Spoke};
use crate::generate::{Defect, GenerateFailure, GeneratedMutant, GeneratedMutantsResponse};

/// What an answer amounted to, when it was not mutants.
pub(super) enum Answered {
    /// Nothing another ask would fix.
    Fatal(GenerateFailure),
    /// Something the model could be told about and asked again.
    Correctable(Correctable),
}

/// A defect worth putting back in front of the model.
pub(super) enum Correctable {
    /// The answer was not the JSON this contract reads.
    NotTheContract(String),
    /// It was, and held a number of mutations nobody asked for.
    WrongCount(usize),
}

impl Correctable {
    /// The defect as the model is told about it.
    pub(super) fn stated(&self) -> Defect {
        match self {
            Self::NotTheContract(reason) => Defect {
                defect: "answer_is_not_the_contract".to_owned(),
                detail: reason.clone(),
            },
            Self::WrongCount(received) => Defect {
                defect: "wrong_number_of_mutants".to_owned(),
                detail: format!("{received} mutations were proposed"),
            },
        }
    }

    /// The failure it becomes once the retry has been spent.
    pub(super) fn failure(self, requested: usize) -> GenerateFailure {
        match self {
            Self::NotTheContract(reason) => GenerateFailure::Unreadable { reason },
            Self::WrongCount(received) => GenerateFailure::WrongCount {
                requested,
                received,
            },
        }
    }
}

/// What one answer about mutations was, in a generation's own terms.
pub(super) fn mutations(
    spoke: Spoke<GeneratedMutantsResponse>,
    wanted: usize,
) -> Result<Vec<GeneratedMutant>, Answered> {
    match spoke {
        Spoke::Answer(answer) if answer.mutants.len() == wanted => Ok(answer.mutants),
        Spoke::Answer(answer) => Err(Answered::Correctable(Correctable::WrongCount(
            answer.mutants.len(),
        ))),
        Spoke::NotTheContract(reason) => {
            Err(Answered::Correctable(Correctable::NotTheContract(reason)))
        }
        Spoke::Refused(reason) => Err(Answered::Fatal(GenerateFailure::Refused { reason })),
        Spoke::Truncated => Err(Answered::Fatal(GenerateFailure::Truncated {
            limit: MAX_COMPLETION_TOKENS,
        })),
        Spoke::NoAnswer => Err(Answered::Fatal(GenerateFailure::NoAnswer)),
    }
}
