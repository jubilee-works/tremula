# Triaging survivors and dismissals

A run's report answers one question — did the suite catch this? — and stops. A
survivor is the answer "no", and it is not yet a finding: a mutation that cannot
change what the program does survives every suite ever written, and a list that
mixes those with real gaps is a list nobody reads twice. `tremula triage` is the
second question, and `tremula dismiss` is where the answer stops being a machine's.

The whole design rests on one measurement. Asked to name the input that separates a
mutation from the original, a model wrote down concrete inputs in exactly the form
it was asked for, and four out of four of the sampled ones were **false** when
executed. So a model's word is never a classification here. What the model is for
is *finding candidate inputs*, which is a search problem it is good at; what
decides is running one.

That produces three classifications, and their names are the honest ones:

| classification | what was established | who decides next |
| --- | --- | --- |
| `distinguished_at_function_level` | one input was executed against both versions of the function and they did different things | a person, first — it is the most useful thing on the list |
| `suspected_equivalent` | a model said the mutation changes nothing, and nothing ran | a person, who verifies it before dismissing it; this is the weakest statement in the document |
| `undecided` | no comparison was made, with a reason saying which of the nine it was | a person, told whether looking again would help |

Two things that classification deliberately does not say. The first name stops at
*function level*: both versions are called directly, so a difference there is not a
claim that the program around them can reach one — two survivors of the measured
sample were separable that way and unreachable in the program, and pretending
otherwise would make the strongest grade the least trustworthy one. And an input
that showed no difference never becomes `suspected_equivalent`: one input failing
to separate two functions is not evidence that none can, and treating it as such
would be believing the very claim it just failed.

Ordinary triage discards nothing, because the two mistakes are not the same size.
A filter that wrongly drops a real gap in a suite destroys the evidence this tool
exists to produce; one that wrongly keeps a harmless mutation costs somebody a
minute. An explicit opt-in can write a separate manifest for a next run:

```sh
tremula triage --model MODEL --exclude-suspected-equivalent \
  --out-manifest PATH
```

Both options are required. The projection removes only non-dismissed entries
classified `suspected_equivalent`. It keeps every other manifest mutant in its
original order, including distinguished, undecided, killed, timed-out, unknown,
untriaged, and already dismissed mutants. This is a risky, reversible run-local
selection, not a finding or a dismissal. The source run's manifest, report, and
snapshot aren't changed. `triage.json` is written as usual, and the destination
must be a new file.

The automatic part therefore establishes what it can, in the order most useful to
read, and retiring a survivor is a person's act: `tremula dismiss` writes the
mutation into `tremula-suppressions.json`, which the project commits, and
afterwards neither `generate` nor `triage` raises it again. Deriving a manifest
doesn't write that record or change its effect.

## What that comes to, measured

Thirty-eight mutations of six modules, hand-labelled one at a time against
`gpt-5.2-2025-12-11`'s answers — sixteen equivalent, twenty-two not — classify like
this:

| label | `distinguished_at_function_level` | `suspected_equivalent` | `undecided` |
| --- | --- | --- | --- |
| not equivalent (22) | 8 | 1 | 13 |
| equivalent (16) | 2 | 8 | 6 |

The two limits above are two of those cells, in numbers. The two equivalent mutations
distinguished at function level are the unreachable-guard case: their only caller never
builds the input that separates them, and a probe calls the function without that
caller. The one not-equivalent mutation called `suspected_equivalent` is a real miss —
an interval rewrite whose only separating inputs are zero-length ranges — and it is why
that classification retires nothing and asks a person to verify it.

The frozen sources, the labels and the recorded answers are in
`tests/fixtures/labelled-mutations/`, and the matrix above is pinned cell by cell — in
both directions, so an improvement fails too — by
`cargo test --test judge_accuracy --features judge_accuracy`, which replays the
recordings and makes no call. The cells that disagree with the labels are asserted by
mutation, with the reason each one is accepted beside it, so a *different* mutation
landing there fails rather than passing as the same number.

## The order of a triage

1. **Read the dismissals**, before anything else. A record that cannot be read is a
   failure worth having before the first model call rather than after the last one.
2. **Find the project's Python and negotiate**, requiring `spans` *and* `probe` on
   top of what a run requires. Discovering one call in that the pack cannot run an
   input would have cost a call per survivor to learn.
3. **Read the [run directory](run-lifecycle.md), and nothing else.** Its `report.json` says which
   mutants survived, its `manifest.json` says what each one replaces, and its
   `snapshot/` holds the bytes the run was measured against. Reading the working
   tree instead would make a triage mean something different depending on what had
   been edited since, and a survivor's evidence would go stale invisibly. Three
   ways for those documents to disagree — a report about another run, a verdict on a
   mutant the manifest lacks, a snapshot holding other bytes — are refused here
   rather than discovered as a puzzling classification.
4. **Per survivor: ask, then run what the answer offers.** The function the
   mutation lands in is cut from the snapshot and sent on its own, because what a
   model needs in order to find a reachable input is the guards between the
   arguments and the changed expression. A claim of equivalence ends there, as a
   suspicion. A claim of difference with an input is put to the pack's `probe`,
   which runs both versions and reports what each did.
5. **Write `triage.json` beside the report**, which is not rewritten. With the
   paired opt-in flags, then derive a separate user-named manifest by removing
   only non-dismissed `suspected_equivalent` entries. The source
   manifest, report, and snapshot remain immutable; `triage.json` records the
   classification. The documents answer different questions, and a reader has to
   be able to tell which is which.

The exit code is `0` whenever the judging happened, whatever it decided — a triage
that established nothing about anything is information, not a failure — and `2`
when it could not happen: no run to read, documents of two runs, no key, or every
single survivor failing for a reason that was about the tools. It is not a gate.

## Running one input, and what that is worth

`probe` is the pack's, because everything it needs to know is language knowledge:
what a literal is, how a module is loaded, what it means for two values to be
equal. The core hands it a file, a span, a replacement and one call; it answers
with what each version did.

The care is all in refusing to report a difference it did not see. Each version
runs three times in a subprocess of its own with `PYTHONHASHSEED` pinned, because a
version that disagrees with itself cannot be compared with anything. Values are
compared by type and by a rendering built from their parts, so a set does not
differ for having been built in another order; `nan` equals `nan`; an enum member
compares by name, which is what lets two loadings of one module be compared at all;
a value whose type compares by identity is undecided rather than guessed at.
Exceptions are compared by type and message and never by traceback. What either
version prints is captured and compared.

The witness itself is never executed as code — every argument is read as a literal,
and a call with anything else in it is refused. The mutated function's own body
*does* run, which is exactly what a run does when it applies a mutant and calls the
suite: the threat model is that one, unchanged, and `contracts/pack-protocol.md`
says so rather than implying a sandbox nobody built.

## Why a dismissal is keyed by the mutation

A mutant's identifier is derived from the file's hash, so it changes when anything
anywhere in that file changes. A decision keyed by identifier would be orphaned by
the next unrelated edit, and the survivor somebody dismissed would return under a
new name. So the key is the file, the text replaced, and the text put there — what
a person actually decided about — and the identifier is kept beside it as
provenance only.

A generation drops a dismissed candidate immediately after the round has finished
asking and before anything about the round is counted, so the manifest, the
console's tally and the exit code all describe the same set of mutants. A decision
whose text is no longer in the file is counted, reported as stale, and kept: it may
be stale because the code moved on or because the reader is on a branch where it
has not, nothing here can tell those apart, and silently discarding a person's
decision is the one thing a record of decisions must not do.
