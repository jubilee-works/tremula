//! One survivor, from a question to a classification.
//!
//! The whole of the reasoning is in [`assess`], and it is short on purpose. There is
//! exactly one path to [`Classification::DistinguishedAtFunctionLevel`] — a witness
//! ran and the two versions did different things — and every other path ends
//! somewhere a person still has to look. In particular a witness that ran and found
//! nothing does **not** become `suspected_equivalent`: the model claimed a
//! difference, one input failed to show it, and neither of those is evidence that no
//! input could.

use tremula_contracts::{
    probe::{ProbeOutcome, ProbeReport, Undecided as ProbeUndecided},
    triage::{Claim, Classification, TriageEntry, Undecided, Witness},
};

use crate::{
    generate::{
        Attempt,
        judge::{
            Claim as Answered, EquivalenceJudge, JudgeFailure, JudgementRequest, Witness as Offered,
        },
    },
    pack::{self, PackError},
    python_env::PythonEnv,
    triage::{failures::TriageFailure, inputs::Survivor},
};

/// What assessing one survivor produced, and what it cost.
#[derive(Debug)]
pub struct Assessed {
    /// The entry as the document will carry it.
    pub entry: TriageEntry,
    /// Every attempt the judge was charged for, the failed ones included.
    pub attempts: Vec<Attempt>,
    /// The model that answered, as it named itself. Absent when none did.
    pub model_resolved: Option<String>,
    /// What went wrong with the tools rather than with the mutation, when
    /// something did. A triage where every survivor carries one of these has
    /// established nothing and says so through its exit code.
    pub infrastructure: Option<String>,
}

/// Ask about one survivor, run whatever it offers, and classify the result.
///
/// # Errors
///
/// Returns [`TriageFailure`] only for a failure that will repeat for every other
/// survivor too — no key, or a key the provider will not take. Everything else is
/// recorded on the entry: a survivor nobody could judge is `undecided`, and a
/// triage that judged nothing is decided by the caller from the count of those.
pub fn assess(
    judge: &dyn EquivalenceJudge,
    env: &PythonEnv,
    snapshot: &std::path::Path,
    survivor: &Survivor,
) -> Result<Assessed, TriageFailure> {
    if survivor.function.is_empty() {
        return Ok(undecided(
            survivor,
            Undecided::NoJudgement,
            "the mutation is not inside any function the language pack reports, so there \
             was nothing to ask about"
                .to_owned(),
            None,
        ));
    }
    let outcome = judge.judge(&JudgementRequest {
        file: survivor.mutant.file.clone(),
        source: survivor.function.clone(),
        original: survivor.mutant.original.clone(),
        replacement: survivor.mutant.replacement.clone(),
    });
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(failure) => {
            if let JudgeFailure::MissingCredential { .. }
            | JudgeFailure::CredentialRejected { .. } = failure.kind
            {
                return Err(TriageFailure::NoCredential {
                    reason: failure.kind.to_string(),
                });
            }
            let said = failure.kind.to_string();
            let mut assessed = undecided(
                survivor,
                Undecided::NoJudgement,
                format!("no judgement was obtained: {said}"),
                None,
            );
            assessed.attempts = failure.attempts;
            assessed.infrastructure = Some(said);
            return Ok(assessed);
        }
    };
    let claim = match outcome.judgement.claim {
        Answered::Distinguishable => Claim::Distinguishable,
        Answered::Equivalent => Claim::Equivalent,
    };
    let offered = outcome.judgement.witness.as_ref().map(recorded);
    let mut assessed = match (claim, outcome.judgement.witness.as_ref()) {
        (Claim::Equivalent, _) => suspected(survivor),
        (_, None) => undecided(
            survivor,
            Undecided::NoWitness,
            "the model said the two differ and named no input a probe could run".to_owned(),
            None,
        ),
        (_, Some(witness)) => run(env, snapshot, survivor, witness),
    };
    assessed.entry.claim = Some(claim);
    assessed.entry.witness = offered;
    assessed.attempts = outcome.attempts;
    assessed.model_resolved = Some(outcome.model_resolved);
    Ok(assessed)
}

/// Run the witness, and let what happened decide.
fn run(
    env: &PythonEnv,
    snapshot: &std::path::Path,
    survivor: &Survivor,
    witness: &Offered,
) -> Assessed {
    let probed = pack::probe(
        env,
        &pack::ProbeRequest {
            root: snapshot,
            file: &survivor.mutant.file,
            span: survivor.mutant.span,
            replacement: &survivor.mutant.replacement,
            call: &witness.call,
        },
    );
    let report = match probed {
        Ok(report) => report,
        // A pack that cannot run a witness is a fact about the tools, and it will
        // be the same fact for the next survivor. It is recorded rather than
        // returned so that the survivors it does not stop are still judged.
        Err(failure) => {
            let said = describe(&failure);
            let mut assessed = undecided(
                survivor,
                Undecided::NoJudgement,
                format!("the witness could not be run: {said}"),
                None,
            );
            assessed.infrastructure = Some(said);
            return assessed;
        }
    };
    from_probe(survivor, report)
}

/// What one probe's answer classifies the survivor as.
fn from_probe(survivor: &Survivor, report: ProbeReport) -> Assessed {
    match report.outcome {
        ProbeOutcome::Differs => Assessed {
            entry: TriageEntry {
                classification: Classification::DistinguishedAtFunctionLevel,
                undecided: None,
                detail: format!(
                    "`{}` told the two apart: {} against {}",
                    report.call,
                    said(report.original.as_ref()),
                    said(report.mutant.as_ref())
                ),
                probe: Some(report),
                ..blank(survivor)
            },
            attempts: Vec::new(),
            model_resolved: None,
            infrastructure: None,
        },
        ProbeOutcome::Indistinguishable => {
            let detail = format!(
                "`{}` did not tell the two apart, which is not evidence that nothing would",
                report.call
            );
            undecided(
                survivor,
                Undecided::WitnessShowedNoDifference,
                detail,
                Some(report),
            )
        }
        ProbeOutcome::Undecided | ProbeOutcome::Unknown => {
            let why = report.undecided.map_or(Undecided::Unknown, translate);
            let detail = format!("`{}` could not be compared: {}", report.call, reason(why));
            undecided(survivor, why, detail, Some(report))
        }
    }
}

/// The probe's reason for making nothing of a witness, in this document's terms.
///
/// A separate vocabulary rather than the same one reused, because these two lists
/// are owned by two sides of a contract: the probe reports what a language could not
/// do, and a triage reports what could not be established — and it has reasons of
/// its own, for the witnesses no probe was ever asked about.
fn translate(undecided: ProbeUndecided) -> Undecided {
    match undecided {
        ProbeUndecided::Nondeterministic => Undecided::Nondeterministic,
        ProbeUndecided::Incomparable => Undecided::Incomparable,
        ProbeUndecided::UnsafeWitness => Undecided::UnsafeWitness,
        ProbeUndecided::Method => Undecided::Method,
        ProbeUndecided::NoSuchFunction => Undecided::NoSuchFunction,
        ProbeUndecided::TimedOut => Undecided::TimedOut,
        ProbeUndecided::Unknown => Undecided::Unknown,
    }
}

/// What each reason means to somebody deciding what to look at next.
#[must_use]
pub fn reason(undecided: Undecided) -> &'static str {
    match undecided {
        Undecided::NoWitness => "no input was named",
        Undecided::WitnessShowedNoDifference => "the input named showed no difference",
        Undecided::Nondeterministic => "a version does not agree with itself",
        Undecided::Incomparable => "the result cannot be compared across processes",
        Undecided::UnsafeWitness => "the input named is not one a probe evaluates",
        Undecided::Method => "the input named a method, which has no receiver",
        Undecided::NoSuchFunction => "the input named a function the module does not define",
        Undecided::TimedOut => "a version did not finish",
        Undecided::NoJudgement => "nothing could answer about it",
        Undecided::Unknown => "for a reason this version does not name",
    }
}

/// One side of a probe, in a few words.
fn said(observation: Option<&tremula_contracts::probe::Observation>) -> String {
    let Some(observation) = observation else {
        return "nothing observed".to_owned();
    };
    let what = observation
        .value
        .clone()
        .or_else(|| {
            observation
                .message
                .as_ref()
                .map(|message| format!("{}: {message}", type_of(observation)))
        })
        .unwrap_or_else(|| type_of(observation));
    if observation.stdout.is_empty() {
        return what;
    }
    format!("{what} (and printed {:?})", observation.stdout)
}

/// The type an observation names, or a stand-in when it names none.
fn type_of(observation: &tremula_contracts::probe::Observation) -> String {
    observation
        .type_name
        .clone()
        .unwrap_or_else(|| "something".to_owned())
}

/// A pack failure as one line, without the paragraph its own message carries.
fn describe(failure: &PackError) -> String {
    match failure {
        PackError::Reported { code, message, .. } => format!("{message} ({code})"),
        other => other.to_string(),
    }
}

/// The entry every classification starts from: the mutation, and nothing decided.
fn blank(survivor: &Survivor) -> TriageEntry {
    TriageEntry {
        mutant_id: survivor.mutant.id.clone(),
        file: survivor.mutant.file.clone(),
        span: survivor.mutant.span,
        original: survivor.mutant.original.clone(),
        replacement: survivor.mutant.replacement.clone(),
        classification: Classification::Undecided,
        undecided: None,
        claim: None,
        witness: None,
        probe: None,
        detail: String::new(),
    }
}

/// A survivor a model called equivalent, which nothing checked.
fn suspected(survivor: &Survivor) -> Assessed {
    Assessed {
        entry: TriageEntry {
            classification: Classification::SuspectedEquivalent,
            detail: "the model said the mutation cannot change what the function does, and \
                     nothing ran to check that"
                .to_owned(),
            ..blank(survivor)
        },
        attempts: Vec::new(),
        model_resolved: None,
        infrastructure: None,
    }
}

/// A survivor nothing was established about, and why.
fn undecided(
    survivor: &Survivor,
    why: Undecided,
    detail: String,
    probe: Option<ProbeReport>,
) -> Assessed {
    Assessed {
        entry: TriageEntry {
            classification: Classification::Undecided,
            undecided: Some(why),
            detail,
            probe,
            ..blank(survivor)
        },
        attempts: Vec::new(),
        model_resolved: None,
        infrastructure: None,
    }
}

/// The witness as the document records it.
fn recorded(offered: &Offered) -> Witness {
    Witness {
        call: offered.call.clone(),
        expect_original: offered.expect_original.clone(),
        expect_mutant: offered.expect_mutant.clone(),
    }
}
