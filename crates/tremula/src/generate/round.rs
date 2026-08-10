//! One function's round of generation: ask, check, ask once more, keep what
//! survived.
//!
//! Three things check a proposal, and they are in this order because each makes
//! the next one meaningful. The search settles where the mutation lands. The rules
//! a manifest keeps whatever the language says — no carriage return, no NUL, a
//! replacement that is not the text it replaces — are settled in the same breath.
//! Then the project's own language pack is asked whether the file still compiles
//! with the replacement in it, whether the replacement can stand for a single
//! node, and whether the span lines up with something its backend can match.
//!
//! Everything any of them refuses is put back in front of the model, once. That
//! the pack's refusals travel the same way as the search's is measured rather than
//! symmetric: a schema-constrained answer practically never breaks its schema — 48
//! measured calls, no retries — so the one correction a round owns is worth nothing
//! unless it is spent on the defects only a file and a parser can reveal.
//!
//! The ask that follows is for the failed slots alone. A model that answered four
//! times and was wrong once is asked for one, and the three that landed are not
//! asked about again.

use std::collections::BTreeMap;

use time::OffsetDateTime;
use tremula_contracts::manifest::Mutant;

use crate::{
    generate::{
        Attempt, CorrectionFeedback, Defect, GenerateError, GeneratedMutant, GenerationRequest,
        MutantGenerator,
        enrich::{SourceFile, Stamp, Target, enrich, with_the_closing_newline},
    },
    pack::PackError,
    provenance,
};

/// What a language pack calls a span its backend cannot match.
///
/// The one refusal a caller can repair for itself rather than pass on. A span the
/// backend cannot match is ordinarily the model's mistake — a bare `if` header is
/// not a node any manifest can replace — but once it is the caller's: a compound
/// statement's node ends after the newline that closes it, and a search for the
/// statement's text stops before that newline. So the newline is taken in and the
/// pack is asked again, and only if it refuses a second time does the model hear
/// about it. The name is the pack protocol's, which publishes the code each of its
/// checks fails with.
const SPAN_MATCHES_NO_NODE: &str = "span_matches_no_node";

/// The check an enriched mutant has to pass in the project it belongs to.
///
/// A closure rather than a type of its own: the only real one starts the project's
/// language pack in a subprocess, and everything that tests this loop has to be
/// able to answer without one. `Ok(None)` passed; `Ok(Some(defect))` is a mutation
/// the language refuses, which is correctable; `Err` is the check itself failing,
/// which is nobody's mistake and must not be corrected as one.
pub type Inspect<'a> = &'a dyn Fn(&Mutant) -> Result<Option<Defect>, PackError>;

/// Everything one function's round needs.
pub struct Round<'a> {
    /// Where the mutations come from.
    pub generator: &'a dyn MutantGenerator,
    /// The file the function lives in, read once.
    pub file: &'a SourceFile,
    /// The function, and where inside it a mutation may land.
    pub target: &'a Target,
    /// The project's own check.
    pub inspect: Inspect<'a>,
}

/// What one round produced, whether or not it finished.
#[derive(Debug, Default)]
pub struct Gathered {
    /// The mutants that survived every check, in the order they were proposed.
    pub mutants: Vec<Mutant>,
    /// How many mutations the model proposed, over both asks.
    pub proposed: usize,
    /// How many proposals each kind of defect refused, by the defect's own name.
    pub refused: BTreeMap<String, usize>,
    /// How many proposals were a mutation this round already had. Two proposals
    /// can be one mutant — the identifier is derived from the span and the
    /// replacement — and a manifest that carried it twice would be refused whole.
    pub duplicates: usize,
    /// The model that answered, as it named itself. Absent when none did.
    pub model_resolved: Option<String>,
    /// Every attempt the generator was charged for, the failed ones included.
    pub attempts: Vec<Attempt>,
    /// Why the round stopped early, when it did. Everything above is still what
    /// was gathered before that happened, and is meant to be used.
    pub failure: Option<RoundFailure>,
}

/// Why a round stopped before it had finished asking.
///
/// Neither of these is a defect of a mutation, which is what separates them from
/// everything in [`Gathered::refused`]: no correction would help, and a caller
/// that reported them as refused proposals would be blaming the model for the
/// provider being down.
#[derive(Debug, thiserror::Error)]
pub enum RoundFailure {
    /// The generator could not answer.
    #[error(transparent)]
    Generator(#[from] GenerateError),
    /// The project's own check could not be made.
    #[error(transparent)]
    Pack(#[from] PackError),
}

impl Round<'_> {
    /// Ask about this function, check every answer, and ask once more about what
    /// was wrong.
    ///
    /// Never fails as a whole. A generator or a check that gives out is recorded in
    /// [`Gathered::failure`] with everything gathered up to that point still in
    /// hand — a first answer whose mutants are checked and good is work already
    /// paid for, and throwing it away because a second call was rate limited would
    /// charge the caller twice for it.
    #[must_use]
    pub fn run(&self, request: &GenerationRequest) -> Gathered {
        let mut gathered = Gathered::default();
        let Some(defects) = self.ask(request, &mut gathered) else {
            return gathered;
        };
        if defects.is_empty() {
            return gathered;
        }
        let again = GenerationRequest {
            mutant_count: defects.len(),
            feedback: Some(CorrectionFeedback { defects }),
            ..request.clone()
        };
        self.ask(&again, &mut gathered);
        gathered
    }

    /// Make one ask and account for everything it produced.
    ///
    /// Returns the defects worth correcting, or nothing when the round cannot go
    /// on — which is recorded on `gathered` rather than returned, because a round
    /// that stopped and a round that had nothing to correct are answered the same
    /// way by the caller.
    fn ask(&self, request: &GenerationRequest, gathered: &mut Gathered) -> Option<Vec<Defect>> {
        let outcome = match self.generator.generate(request) {
            Ok(outcome) => outcome,
            Err(failure) => {
                gathered.attempts.extend(failure.attempts.iter().cloned());
                gathered.failure = Some(RoundFailure::Generator(failure));
                return None;
            }
        };
        let stamp = Stamp {
            model: outcome.model_resolved.clone(),
            generated_at: provenance::timestamp(OffsetDateTime::now_utc()),
        };
        gathered.model_resolved = Some(outcome.model_resolved);
        gathered.attempts.extend(outcome.attempts);
        gathered.proposed += outcome.mutants.len();
        let mut defects = Vec::new();
        for candidate in &outcome.mutants {
            match self.judge(candidate, &stamp) {
                Err(failure) => {
                    gathered.failure = Some(RoundFailure::Pack(failure));
                    return None;
                }
                Ok(Ok(mutant)) => {
                    if !keep(&mut gathered.mutants, mutant) {
                        gathered.duplicates += 1;
                    }
                }
                Ok(Err(defect)) => {
                    *gathered.refused.entry(defect.defect.clone()).or_default() += 1;
                    defects.push(defect);
                }
            }
        }
        Some(defects)
    }

    /// One proposal, taken as far as it goes.
    ///
    /// The nesting says which failure is which: the outer result is the check
    /// itself failing, and the inner one is the verdict on the mutation. Collapsing
    /// them would make a language pack that would not start look like a model that
    /// answered badly.
    fn judge(
        &self,
        candidate: &GeneratedMutant,
        stamp: &Stamp,
    ) -> Result<Result<Mutant, Defect>, PackError> {
        let mutant = match enrich(candidate, self.file, self.target, stamp) {
            Ok(mutant) => mutant,
            Err(defect) => return Ok(Err(defect)),
        };
        let Some(defect) = (self.inspect)(&mutant)? else {
            return Ok(Ok(mutant));
        };
        if defect.defect != SPAN_MATCHES_NO_NODE {
            return Ok(Err(defect));
        }
        let Some(snapped) = with_the_closing_newline(&mutant, self.file) else {
            return Ok(Err(defect));
        };
        match (self.inspect)(&snapped)? {
            None => Ok(Ok(snapped)),
            Some(again) => Ok(Err(again)),
        }
    }
}

/// Add `mutant` unless the round already has it. False means it was a repeat.
fn keep(kept: &mut Vec<Mutant>, mutant: Mutant) -> bool {
    if kept.iter().any(|already| already.id == mutant.id) {
        return false;
    }
    kept.push(mutant);
    true
}
