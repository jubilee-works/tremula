# 3. The report is the product

## Status

Accepted

## Context

A mutation testing run ends with a list of mutants the suite failed to detect.
The obvious next step is to write the missing tests, and it is tempting to build
that step into the tool: find the gap, generate a test, prove the gap closed.

That step is a different product. Writing a test that a human will accept into
their suite requires understanding the project's conventions, its fixtures, its
naming, and its review culture. Tools that attempt it acquire a sandbox, a
confirmation protocol, a code-writing dependency, and an enormous surface for
being subtly wrong in a way that survives review.

There is also a structural problem: a system that both proposes mutants and
writes the tests that kill them is grading its own work.

Meanwhile the evidence itself — this exact mutation, at this exact location,
survived this exact suite, and here is the diff and the run that proves it — is
both the hard part and the part nobody else produces.

## Decision

The product ends at verified evidence: a report of per-mutant verdicts, and
later an evidence bundle containing the patch, the execution record, and the
provenance for each mutant. Test generation is **permanently out of scope**.
Closing the gaps is the job of whoever consumes the bundle, including a coding
agent driven by the user.

Two rules follow from taking the evidence seriously.

**Judgement belongs to the core.** A language pack reports signals — how many
tests passed, failed, and errored, how the runner exited, whether collection
worked, whether the collected test set matched the baseline. It never reports a
verdict. One implementation of the rules means one definition of "killed".

**Aggregation is conservative.** A mutant that breaks an import, so that
collection fails before any test body runs, is *not* counted as killed. It would
be detected by literally any test suite, so counting it would inflate the score
in a way that says nothing about test quality. It is excluded and reported
instead. The same reasoning excludes a run whose collected test set drifted from
the baseline: nothing was proven, so nothing is claimed.

## Consequences

There is no sandbox, no confirmation protocol, and no dependency on a
code-writing model in the judgement path. The tool that reports a gap does not
also get to declare it closed.

Scores undercount rather than overcount. A manifest whose mutants all break
imports produces no usable verdict at all, and the run exits with a failure
rather than a flattering number — an investigation signal instead of a score.

Because the report is the deliverable, it has to be good enough to act on
without rerunning anything: a stable machine-readable document, a location for
each mutant, and the caveats that must accompany any presentation of the
results. In particular, a survived mutant is never presented as proof of a
missing test. It means the existing suite did not detect it, and the report says
so in its own text: coverage is unverified, and some survivors may be equivalent
mutants that no test could ever detect.
