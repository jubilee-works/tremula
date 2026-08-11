# Contracts

Everything that crosses the boundary between the tremula core and a language
pack is a JSON document with a published schema. This chapter explains what each
document is for and the semantics a producer has to get right. The schemas
themselves live in `contracts/schemas/`, are generated from the core's types,
and are the authority whenever this page and they disagree.

## The seven documents

| Document | Written by | Read by | Answers |
| --- | --- | --- | --- |
| `manifest` | whoever decides the mutations | core, pack | What should be mutated, and where? |
| `spans` | pack | whoever decides the mutations | Where are this file's functions, and what inside them is not a target? |
| `capabilities` | pack | core | Which contract version and subcommands does this pack support? |
| `baseline` | pack | core | What did the suite do with no mutation applied? |
| `results` | pack | core | What happened to each mutant, in neutral terms? |
| `report` | core | machines, people | What is the verdict, the score, and the exit code? |
| `pack-error` | pack | core | Where did the pack stop, and why? |

Only the report contains verdicts. A pack reports signals and never judges; the
core judges and never touches a backend. The command lines that carry these
documents between the two are specified in `contracts/pack-protocol.md`.

`contracts/schemas/` also holds schemas for documents that cross other boundaries —
`probe`, `triage`, `suppressions`, and `bundle`. They are published on the same
terms and for the same reason, and **The bundle index** below is about the one of
them that leaves this repository entirely.

The manifest, baseline, results, and report each carry a `schema_version`, and
the capabilities document reports the same value under the name
`contract_version`. The pack error document has no version field of its own: it
is a last-line failure signal, and the contract version it belongs to is
established by the capability handshake before any work starts.

The documents produced by one run also carry that run's `run_id`, so a stale
baseline paired with fresh results is detectable rather than silently averaged
in.

## Manifest semantics

A manifest entry says: in `file`, replace the bytes of `span` with
`replacement`. Three details are easy to get wrong.

**Spans are raw byte offsets.** `span` is `[start_byte, end_byte)` — 0-indexed,
end exclusive — over the same raw bytes that `base_file_sha256` hashes, not over
characters and not over a decoded string. `original` is the UTF-8 decoding of
exactly those bytes, with no newline translation, and validation compares byte
slices rather than strings.

**`start_byte` must be below `end_byte`.** A mutant replaces existing code;
insertion at a point is not expressible in this contract. One mutant is one
contiguous span, and that invariant cannot be broken without raising
`schema_version`.

**Only plain UTF-8 with LF line endings is supported.** A byte-order mark, a
non-UTF-8 encoding declaration in the first two lines, or a CRLF line ending
causes the file to be rejected before anything runs. CRLF is rejected rather
than handled because tooling that reads source with universal newline
translation sees different bytes than the manifest measured, so byte spans drift
by one per preceding line and multi-line originals can never match.

The same rule applies to `replacement`: a carriage return anywhere in it is
rejected. It is checked here rather than left to a language pack because what a
carriage return does depends on the shape of the span it lands in, and no single
pack-level check catches it. Measured on the Python pack: in a replacement for an
expression the carriage return disappears, in a replacement for a whole statement
it survives into the file as a CRLF line ending, and inside a string literal it
can change how many statements the replacement parses as. Carriage returns
interact unpredictably with a backend's parse-and-reshape pipeline, so they are
rejected outright instead of being reshaped into something the manifest never
described.

An empty `mutants` list is a valid manifest and means "nothing to test".

## Mutant identifiers

A mutant's `id` is derived from its own content, so that any producer — a
generator, a second execution backend, a consumer of the evidence — computes the
same identifier for the same mutation:

```
sha256("tremula/mutant/v1" NUL file NUL start_byte NUL end_byte NUL
       base_file_sha256 NUL replacement)
```

The parts are UTF-8 encoded and joined by single NUL bytes. Both offsets are
rendered as unpadded, unsigned decimal ASCII, so byte 412 contributes the three
bytes `412`. The result is written as lowercase hexadecimal.

`base_file_sha256` is part of the derivation, so editing the target file changes
every identifier in it — which is what makes resuming an interrupted run safe.
`provenance` is deliberately excluded, so recording how a mutant was generated
never changes its identity. It is free-form, and the keys a generator writes into
it are conventional rather than validated; `contracts/pack-protocol.md` names
them.

The core recomputes the identifier during validation and rejects a manifest
whose identifier does not match. Two mutants that derive the same identifier are
the same mutation twice, and are rejected as a duplicate rather than executed.

## Validation

Validation is split in two, along the line that decides who has to know about a
language.

The **core** checks bytes, hashes, and paths: `file` is a plain relative POSIX
path inside the project, the file exists, its SHA-256 matches
`base_file_sha256`, the file is plain UTF-8 with LF endings, the span fits and
is non-empty, the bytes at the span equal `original`, `replacement` differs from
`original`, the identifier is canonical, and no identifier repeats. A
replacement above 10 KB is a warning, not an error.

The **pack** checks the language: that the file still compiles with the
replacement spliced into the span, that the replacement can stand in for a single
node without losing text, that the span corresponds to a node the backend can
actually match, and that the mutated file's syntax tree differs from the
original's. A pack reports which of these checks it implements through
`validate_checks` in its capabilities document, so a newer core can tell whether
the checks it wants are available.

A replacement is judged by the file it makes rather than on its own, which is
measured: of 144 proposals in one sweep, 34 were refused for not parsing alone and
every one of them compiled once spliced in, while two that parsed alone broke the
file they went into. Sixteen of the 34 are still refused, by arity and by the node
boundary, because a bare compound-statement header is not a node any manifest can
replace. And a compound statement's node ends *after* the newline that closes it,
so a span aimed at one takes that newline in — into `original` as well, since the
two are the same bytes.

A mutant the pack refuses during a `run` is left out of the run rather than ending
it: it is reported as `not_applied` with the reason under `backend_raw.refusal`,
and only a manifest whose every mutant is refused fails outright.
`contracts/pack-protocol.md` has the whole rule and the three places a refusal can
come from.

## Verdicts

Verdicts are decided by the core from the neutral signals in `results`, matching
the first rule that applies:

| Order | Condition | Verdict |
| --- | --- | --- |
| 1 | `execution_status` is `skipped` | `skipped` |
| 2 | `execution_status` is `not_applied` | `not_applied` |
| 3 | `execution_status` is `not_run` | `not_run` |
| 4 | `execution_status` is `backend_error` | `runtime_error` |
| 5 | `execution_status` is `timeout`, or the runner reports `timed_out` | `timeout` |
| 6 | no `runner` signals at all | `runtime_error` |
| 7 | `collect_error`, or `exit_class` is `interrupted` or `infra_error` | `runtime_error` |
| 8 | `failed` or `errors` above zero | `killed` |
| 9 | `passed` at least 1, `exit_class` is `ok`, and both `collected` and `collected_ids_hash` match the baseline | `survived` |
| 10 | anything else | `runtime_error` |

Three of these rows carry decisions worth stating plainly.

**A timeout counts as a detection.** A mutant that makes the suite hang has
changed observable behaviour. It is aggregated into `killed`, counted separately
in `timeout`, and never on its own causes a failing exit code — but any timeout
at all produces a warning, because a timeout is also what an overloaded machine
looks like.

**A failure and an error both count as a kill, a collection error does not.** A
test that fails an assertion and a test that raises in a fixture have both
*run* and reacted to the mutant. A mutant that breaks an import stops collection
before any test body executes; every suite would "detect" it, so counting it
would inflate the score without saying anything about test quality. A kill
caused only by errors is labelled `killed_by_error` so the distinction survives
into the report.

**Row 9 requires the same test set as the baseline, not merely a green run.**
Without that requirement, a suite that collected nothing — a mistaken working
directory, a broken selection — reports "everything passed" and every mutant
appears to survive. Comparing both the count and the hash of the collected test
identifiers catches a set that changed size and a set that changed membership.

The report must contain exactly one verdict per manifest mutant. A missing,
surplus, or repeated result is an adapter defect, and produces a failure rather
than a report.

## The bundle index

`bundle.json` is the one document in `contracts/schemas/` that no pack reads. It
crosses a different boundary: out of this repository altogether, to whoever a run's
evidence was handed to. That makes its reader the furthest away of any, and the
`schema_version` and must-ignore rules matter here most.

It is an **index and not a summary**. It carries no verdict and no count of its
own, for the reason `triage.json` carries no verdict either: a copy is a second
thing that can be wrong, and a bundle damaged in transit or uploaded in part would
disagree with itself in a way no consumer could detect. Consumers read
`documents.report` and check its `sha256`.

| Field | What it is for |
| --- | --- |
| `documents` | the four contract documents, plus `triage` when the run was triaged; each `{path, sha256}` |
| `base` | `revision` (the run's `observed_revision`, `null` outside a checkout), `dirty`, and `reproducible_from_revision` — which is `revision != null && !dirty` |
| `suite` | `tests`, the selectors the run was given in the order it was given them, and `collected` from the baseline |
| `attachments` | one per mutant in the report's order: `{id, patch, log, patch_error}`, where `patch` and `log` are `{path, sha256}` or `null` |
| `exposure` | what kinds of content are in the bundle, four independent facts |
| `logs_truncated` | whether any log was shortened |

Three things it deliberately does not have. There is no rendered command line: a
derived string goes stale when a command-line surface changes and nothing fails
when it does, so `suite.tests` is the selectors themselves and whoever renders a
command renders it from those. There is no timestamp of its own — the report's
`started_at` and `finished_at` already have that, and a stamp would make two
bundles of one run differ. And there is no field naming the function a mutation
landed in, because no run artefact records one: the location is the report's `span`
and `location`, and a name is in `triage.json` when a triage was run.

**`exposure` is four booleans and not one.** A single "safe" flag would be a false
label, since a default bundle carries source context inside every patch and test
code inside every log. `patch_context` says patches are present, `standalone_logs`
says log files are, `absolute_paths` says `report.json` still holds the project root
as it was typed, and `backend_raw_output` is **always true** — `results.json` is
carried byte for byte and the execution backend's own output, markers and absolute
paths included, is inside it. Leaving out the log files does not change that, which
is exactly why the two are separate fields.

`logs/baseline.txt` travels but is named by no field: it is not one mutant's, and
the hashes in the index are of the documents and the patches, which is what the
index says they are. `START_HERE.md` sends a reader to it.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | No survivors. Includes a manifest with nothing to test. |
| 1 | At least one mutant survived. |
| 2 | The run cannot be trusted, or it never completed. |

Code 2 is an explicit list, not a catch-all: a run-level failure that stops the
work before verdicts exist (preflight, the baseline gate, the backend
subprocess, or the pack's own error channel); any `not_applied`; any `not_run`;
a report that does not cover the manifest exactly; or a manifest with at least
one mutant that produced no usable verdict at all.

Everything outside that list — including individual mutants excluded as
`runtime_error` — leaves the exit code alone. Three survivors alongside one
mutant with an environment problem is exit 1. Three survivors alongside one
mutation that never landed is exit 2, because a mutation that was validated and
then never applied is a defect in tremula, not in the project under test.

## Evolution

Adding a field is a compatible change, and consumers must ignore fields they do
not recognise. Adding a value to an existing enum is compatible on the same
terms, but only where the enum defines an `unknown` value for consumers to fall
back on — a failure's `stage` and an excluded span's `kind` both do. Where it
does not, as with `language`, an unknown value genuinely means "no pack for
this", and rejecting the document is the right answer.

Optional fields may be omitted or sent as `null`, and both mean absent.
Producers also differ over whether an empty optional map such as `provenance` or
`backend_raw` is written as `{}` or left out; both are valid and consumers must
treat them the same.

Version acceptance is deliberately simple while there is only one version:
consumers expect `schema_version` `0.1`, and negotiating anything beyond that is
the job of the capability handshake. A richer rule is worth writing when a
second version exists and not before.
