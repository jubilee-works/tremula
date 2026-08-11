# Architecture overview

tremula is a Rust binary that decides verdicts and a language pack that runs
tests. Neither knows how the other works. The binary never imports pack code and
cannot name a test framework; the pack never decides whether a mutant was caught.
Everything between them is a documented command line and a JSON document with a
published schema.

The reason for the split is that judgement is language-neutral and execution is
not. "The suite failed, so the mutant was caught" is the same sentence in every
language; how a suite is started, how its result is read, and how a byte range
becomes a syntax node are different in each. Putting the second kind of knowledge
in a separate program means a new language is a new pack, not a new fork.

## What each command is for

Four commands, each answering one question and none of them answering another's.
The order they come in is the order the questions arise, and the reason they are
separate commands rather than flags is in the last column.

| Command | Question | Reads | Writes | Why not part of the previous one |
| --- | --- | --- | --- | --- |
| `generate` | what mutations are worth trying? | one source file, its tests, a model | a manifest | it costs money and sends code to a provider; a manifest is also written by hand |
| `run` | did the suite catch them? | a manifest, the project | a run directory | — |
| `triage` | is a survivor a gap or a mutation that changes nothing? | one run directory, a model | `triage.json` beside the report | it costs a model call per survivor, and most runs end at the console |
| `bundle` | can somebody else reproduce this? | one run directory | a bundle directory | `triage` happens *after* a run, so a bundle made by the run would be missing `triage.json` every time |

`validate` and `restore` and `dismiss` sit outside that line: the first checks a
manifest against the bytes on disk without needing anything installed, the second
puts a run's target files back, and the third is where a person's decision about a
survivor is recorded.

The last column's second entry is the whole reason `run --bundle` does not exist.
A bundle is meant to be immutable and is never written over, so a run that made
one automatically would force either a second bundle of the same run or a refusal
to write the complete one. Discoverability is solved instead by a line of console
output at the end of a run naming whichever of `triage` and `bundle` comes next.

## What each side owns

| | Core (`crates/tremula`) | Pack (`packs/python`) |
| --- | --- | --- |
| Reads | manifest, `spans`, `probe`, `results`, `baseline`, the dismissals | manifest, source files |
| Writes | manifest, `report`, `triage`, the dismissals, the console report | `results`, `baseline`, execution logs |
| Owns | validation against bytes on disk, asking a model, turning an answer into spans, the project lock, run directories, snapshots, verdicts, classifications, the exit code | the environment check, where a mutation may land, language-level validation, running one input against two versions of a function, the reference run, applying mutants, running the suite |
| Never | starts a test suite, parses source, evaluates an expression, names a backend | decides a verdict, decides a classification, decides an exit code beyond "worked" or "failed" |

The pack is invoked as `<python> -I -m tremula_python <subcommand>`. Running it as
an installed module rather than a script, through an isolated interpreter, is
what makes the pack the core negotiated with the pack that actually runs: a
directory named `tremula_python` in whatever the user was standing in cannot
answer for it, and neither can `PYTHONPATH`. Every path argument is absolute,
because the pack starts subprocesses with working directories of their own.

## Where mutants come from

A manifest has to be written by something. That something is a `MutantGenerator`:
one function in, one set of proposed mutations out, with the model that answered
and what the call cost. `tremula generate` is what calls it, and **The order of a
generation** below is what it does; the order of a run after that begins with a
manifest that already exists, whether this wrote it or a person did.

The seam is narrow on purpose. A generator is handed the text of a function and
the tests that cover it, and answers with `file`, `original`, `replacement` and
`description` — **no span**. Asking a model for byte offsets was measured against
a real provider and produced usable offsets under one per cent of the time, while
the text it wanted to replace came back findable in the function four times in
five. So the address of a mutation is its text, and turning that into a span is
the core's arithmetic against the file's own bytes, where it was always going to
be correct.

That division decides who retries what. A generator can only see what it was sent
and what came back; everything measured against the file belongs to the caller,
which is why a request can carry the defects a caller found and ask again.

| what went wrong | whose problem | what happens |
| --- | --- | --- |
| the answer is not the JSON the contract asks for, carries a member the contract forbids, or holds a number of mutations nobody asked for | the generator | asked again once, with the defects attached |
| the model refused, the answer stopped at the token limit, or there were no choices at all | the generator | three distinct failures, none of them asked again |
| what came back was not the provider's protocol at all | the generator | a failure of its own, not asked again: a correction is addressed to a model, and no model spoke |
| the provider is rate limiting | the generator | waited out once when the header is there, capped; a header this cannot read still earns the wait, at the cap; no header at all is a failure naming the quota |
| `original` is not in the function, or is in it more than once | the caller | ask again with the defect list |
| the replacement does not compile once spliced into the file | the caller | ask again with the defect list |
| the mutation cannot change what the function does | the caller | discard it — a prompt that forbade this was measured and did not work |

The generator's two retries are counted apart, one each: a call that spent its
wait can still correct an answer, and a call that corrected an answer can still
be told to wait. Every failure message says only what actually happened, so a
message that mentions a retry was reached by a path that spent one.

The answer is held to the contract the request sent. The schema that goes out
forbids a member it does not name, and the reader refuses one, because a receiver
that ignored it would be declining to enforce what it had just asked for — an
answer carrying a `span`, which is what the measured provider sent when the
measured schema asked for offsets, is a correctable defect rather than a field to
skip. That rule is the opposite of the one for the documents in `contracts/`,
where a reader ignores what it does not recognise; the difference is that those
are published documents with versions and this is one exchange with a model that
was just told what to send.

Every attempt is on a ledger the caller gets back, including the attempts that
failed, because a provider charges for those too. A failure carries the same
ledger as a success, so spend can be settled whichever way the call went.

The reference implementation asks OpenAI's chat completions endpoint with the
answer constrained to a schema derived from the type that reads it. The key comes
from `OPENAI_API_KEY` at the moment of the call, travels as a header, and is
taken back out of anything the provider says before that reaches a message.
Which model to ask is the caller's to name, exactly, because the answer's own
report of it is what a mutant records as provenance.

## The order of a generation

1. **Refuse to write over a manifest that is already there.** Before anything is
   spent, because the answer to "where does this go" cannot change later.
2. **Read the file and hold it to every rule a target file keeps.** The same rules
   a manifest's target is held to, asked of a path with no manifest behind it: a
   file this refuses would produce a manifest the neutral validation throws out,
   after a model had been paid for it.
3. **Find the project's Python and negotiate**, requiring `spans` on top of what a
   run requires. A generation has no way to work out where a mutation may land
   without asking; a pack that has never heard of the call still runs a manifest
   somebody else generated, which is why the requirement is here and not there.
4. **Ask the pack where a mutation may land**, and check its report against the
   bytes that were read. A report about another version of the file would place
   every span somewhere it is not.
5. **Pick the named functions out of that report.** A name that is not there is
   refused with the ones that are; a name the file spells twice asks for the span
   of the one that is meant, because a qualified name is for a person to read and
   is never a key.
6. **Per function: ask, check, ask once more.** The function's own source is cut
   from the file at the span the pack reported; the test files named on the command
   line are read and shown alongside it, and the stretches that carry no behaviour
   are named as places not to aim at. Every proposal is turned into a mutant
   against the file's own bytes and then put to the pack one mutant at a time —
   one at a time because the pack's validation stops at its first defect, and a
   defect that is not reported is a correction that cannot be asked for. What
   anything refused is asked again, once, for the failed slots alone.
7. **Read the file's hash again, validate the whole manifest, write it.** The
   second reading is what catches a file that moved under the generation: every
   offset in a manifest is a claim about bytes that were there. The whole-manifest
   validation is the invariant a per-mutant answer cannot establish — no repeated
   identifier, every span still where it was said to be.

Nothing in that list claims the project's lock, and that is a decision. A
generation reads and never patches a source; what a concurrent run could do is
change a file underneath it, and both ways that could go wrong end safely — step 7
catches it and nothing is written, or a manifest that got past would be refused by
the run that tried to use it, since a mutant's identifier is derived from the hash
of the file it was generated against. Taking the lock would instead make
generating and running exclude each other for no gain.

A refused proposal is the ordinary course of asking a model for mutants and does
not fail the command. A manifest with nothing in it does, and so does a function
whose model or whose check gave out — with the manifest still written, because what
the other functions produced is real. The summary names which function stopped and
why, since an exit code cannot.

## Where a survivor goes next

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
| `undecided` | no comparison was made, with a reason saying which of ten it was | a person, told whether looking again would help |

Two things that classification deliberately does not say. The first name stops at
*function level*: both versions are called directly, so a difference there is not a
claim that the program around them can reach one — two survivors of the measured
sample were separable that way and unreachable in the program, and pretending
otherwise would make the strongest grade the least trustworthy one. And an input
that showed no difference never becomes `suspected_equivalent`: one input failing
to separate two functions is not evidence that none can, and treating it as such
would be believing the very claim it just failed.

Nothing is discarded, because the two mistakes are not the same size. A filter
that wrongly drops a real gap in a suite destroys the evidence this tool exists to
produce; one that wrongly keeps a harmless mutation costs somebody a minute. So
the automatic part establishes what it can, in the order most useful to read, and
retiring a survivor is a person's act: `tremula dismiss` writes the mutation into
`tremula-suppressions.json`, which the project commits, and afterwards neither
`generate` nor `triage` raises it again.

### What that comes to, measured

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

### The order of a triage

1. **Read the dismissals**, before anything else. A record that cannot be read is a
   failure worth having before the first model call rather than after the last one.
2. **Find the project's Python and negotiate**, requiring `spans` *and* `probe` on
   top of what a run requires. Discovering one call in that the pack cannot run an
   input would have cost a call per survivor to learn.
3. **Read the run directory, and nothing else.** Its `report.json` says which
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
5. **Write `triage.json` beside the report**, which is not rewritten. The two
   documents answer different questions, and a reader has to be able to tell which
   is which.

The exit code is `0` whenever the judging happened, whatever it decided — a triage
that established nothing about anything is information, not a failure — and `2`
when it could not happen: no run to read, documents of two runs, no key, or every
single survivor failing for a reason that was about the tools. It is not a gate.

### Running one input, and what that is worth

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

### Why a dismissal is keyed by the mutation

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

## The order of a run

The sequence below is mostly forced rather than chosen; each step is either the
reason the next one is safe or the reason it means anything.

1. **Read and parse the manifest.** Its declared `schema_version` has to be one
   this binary reads. Nothing in the project has been touched yet, so nothing can
   have raced.
2. **Claim the project.** `.tremula/lock` records the run's identifier, its
   process id, and — once the pack is working — the pack's process id too. Mutants
   are applied in place and a project has one source tree, so a second concurrent
   run would corrupt the first one's files. The claim is made by linking the lock
   to a document already written, so a lock that exists is always complete and can
   never be read as one nobody holds. A lock whose processes are all gone is taken
   over, with a warning, one run at a time — reading a lock and replacing it are
   separate steps, and turns are taken through `.tremula/lock.gate` so that two
   runs cannot both decide the same lock is abandoned. A lock whose process cannot
   be signalled counts as held, because that is what a process belonging to
   another user looks like.
   A manifest with nothing in it takes the lock too: publishing a newest run
   without holding it would redirect a live run's recovery pointer.
3. **Observe the revision.** `git rev-parse HEAD` and `git status --porcelain`,
   excluding `.tremula`. This happens before any artifact exists, because run
   directories are written inside the project and would otherwise report every
   clean tree as modified. A project that is not a repository records no revision.
4. **A manifest with nothing to test stops here**, with a run directory, a report,
   and exit 0. A change that touched no code is ordinary input, not an error.
5. **Validate against the bytes on disk.** Path spelling, where the path actually
   leads, encoding, hashes, spans, and each identifier's derivation — the checks
   in `crates/tremula/src/validation`. Each target is read exactly once, and those
   bytes are what every check is measured against.
6. **Find the project's Python and negotiate.** The interpreter named on the
   command line, then the active virtual environment, then the project's own
   `.venv`, each resolved to an absolute path where the caller named it — nothing
   afterwards runs from the caller's directory. It has to have the pack installed,
   and the pack has to report a contract version this binary speaks, the
   subcommands a run uses, and at least one check of its own.
7. **Reserve the run directory, write the snapshot, publish it as `latest`.** The
   snapshot is the bytes from step 5 rather than a second reading of the files.
   Publishing before the pack starts is deliberate: the run a reader needs
   `tremula restore` to find is precisely the one that did not finish.
8. **Run the pack.** It performs its own preflight, its language checks, the
   reference run, then applies every mutant and collects the results. Its process
   is named in the lock while it lasts, and no way out of this step leaves it
   running: a pack that outlived the run would go on editing the files the report
   describes. A mutant the pack cannot apply is left out and reported as
   `not_applied` with the reason rather than ending the run — which is why the
   exit code below counts any `not_applied` as untrustworthy: the batch is saved,
   and the fact that something was left out is not hidden. Which mutants were left
   out, and why, is `refusals.json` in the run directory.
   One of those refusals is a rule about mutations rather than about this project,
   and it applies to every manifest: a replacement that leaves the file with the
   syntax tree it already had is refused, whoever wrote it — a person editing a
   manifest by hand as much as a generator — because no test could tell the two
   files apart, and reporting such a mutant as survived would blame the suite for a
   difference that is not there. It arrives as `not_applied`, and so as exit 2.
9. **Read back what it wrote, and check that it belongs.** Both documents have to
   carry this contract version, this run's identifier, and this pack's name,
   version and contract version. A pack that reports success without leaving a
   document is a defect in the pack, and is reported as one.
10. **Judge, write `report.json`, print.** The run is stamped as finished here, at
    the end of its work rather than before it started. The last line of stdout is
    `run_dir=<path>`, absolute, and the process exits with the report's own exit
    code.

## What a run leaves behind

```
.tremula/
├── lock                       # while a run holds the project
├── lock.gate                  # whose turn it is to take an abandoned lock over
└── runs/
    ├── latest -> <run-id>
    └── <run-id>/              # UTC timestamp + the first six hex digits of the manifest's hash
        ├── manifest.json      # the input, copied by the pack
        ├── config.toml        # the backend configuration the pack generated
        ├── session.sqlite     # the backend's own state
        ├── baseline.json      # the reference run
        ├── results.json       # per-mutant signals, in neutral terms
        ├── refusals.json      # which mutants the run left out, and why
        ├── report.json        # verdicts, score, exit code — written by the core
        ├── triage.json        # the survivors, classified — written by `tremula triage`
        ├── snapshot/          # the target files as they were
        ├── pycache/           # bytecode, kept out of the project's own
        └── logs/              # what the pack and the suite said
```

A run directory is reserved, never reused: runs of the same manifest within one
second would otherwise interleave their documents, so each after the first gets a
suffixed name of its own, up to `-9`.

Only some of these exist for a run that failed early. A run that stopped before
its manifest was validated has a report and nothing else; one that stopped in the
pack's reference run has neither the `manifest.json` copy nor `refusals.json`,
because both are written when the pack plans the session. That is normal, not
damage. `triage.json` exists only for a run somebody has triaged, and a run whose
`manifest.json` or `snapshot/` is missing cannot be triaged at all: those two are
where a survivor's mutation and the bytes it was measured against come from.

The project's dismissals are not in here. `tremula-suppressions.json` sits beside
the manifest, is meant to be committed, and outlives every run — which is the point
of it.

## What a bundle is, and what it is not

A run directory is a working space; a bundle is what is handed to somebody else.
Half of the list above is the backend's own state, a generated configuration, a
bytecode cache and the project's sources, and freezing the lot of it would make
`session.sqlite` a published contract. So `tremula bundle` copies out the part that
is evidence, checks it, and writes it somewhere of its own:

```
tremula-bundle-<run-id>/
├── START_HERE.md          # reading order, how to reproduce one bug, what this cannot do
├── bundle.json            # the index: every document and patch, by path and by SHA-256
├── report.json            # the verdicts — byte for byte as the run wrote them
├── manifest.json          # each mutant's span, original text, and replacement
├── results.json           # the neutral execution signals, and the diffs
├── baseline.json          # what the suite did unmutated
├── triage.json            # only when the run was triaged
├── patches/<id>.patch     # one per mutant, each accepted by `git apply --check`
└── logs/<id>.txt          # the suite's output, cleaned, plus logs/baseline.txt
```

Four things are worth knowing about how it is built.

**It is assembled somewhere else first.** Everything goes into `<out>.part`, every
hash is checked against the bytes on disk, and only then is the directory renamed
into place. A bundle is a thing that gets copied and sent, and a half-written one
looks exactly like a whole one. A failure anywhere leaves neither the output path
nor the staging directory.

**A patch is checked before it is published.** The diffs come from
`results.entries[].diff`, which is a published contract, and each one is offered to
`git apply --check` with the run's own `snapshot/` as the working directory — the
snapshot holds the target files under their project-relative paths, so it is
already the tree the diff was made against and no temporary worktree is needed. One
patch per invocation, since git applies a list of them in sequence. A patch git
refuses is not written, and `patch_error` in the index says why; a *survivor* whose
patch is refused also fails the command, because reproducing a survivor is the
whole point. The diff is still in `results.json` either way — `patch: null` means no
file was written, never that anything was withheld.

**The index copies no verdict.** `bundle.json` names `report.json` and its hash
rather than repeating the score, for the reason `triage.json` does not repeat one:
a copy is a second thing that can be wrong, and a bundle damaged in transit or
uploaded in part would disagree with itself in a way nothing could detect.

**Logs are cleaned and then shortened, in that order.** A suite's output names the
interpreter, the directory, and the person whose home both are inside, and carries
the runner's own `TREMULA-RESULT` line. All of it goes — the home directory, the
project root, the run directory and the temporary directory, each in both the
spelling it was given and the one it resolves to, since a log holds whichever the
process that printed it had. Then, and only then, a log over 64 KiB is cut to its
first and last 32 KiB with a note saying how many bytes went. The other order looks
equivalent and is not: a cut in the middle of an absolute path leaves two halves,
and neither half matches anything a replacement is looking for.

What a bundle does not promise is that the suite can be run from it. It carries the
evidence and not the project, which is what `base.revision` is for; `exposure` says
what kinds of content are actually in it, including that `results.json` holds the
backend's raw output whether or not the log files travel.

## Exit codes

For a run: `0` — no mutant survived. `1` — at least one did. `2` — the run cannot
be trusted.

The third is the interesting one, and it is an explicit list rather than a
judgement call: a run-level failure (a lock conflict, a failing reference run, a
pack that could not start), a mutation that was never applied, a mutant that was
scheduled and never ran, results that do not cover the manifest exactly once, or
a manifest with mutants that produced no usable verdict at all. Anything else is
not `2`. One mutant with an environment problem is reported, excluded from the
score, and does not fail the run.

`triage` and `bundle` are not gates and have no `1`. Both report `0` for having
done their work and `2` for not having been able to: for `triage`, no run to read,
documents of two different runs, no key, or every survivor failing for a reason
about the tools; for `bundle`, no run to read, documents that disagree with each
other, a write that failed — or a survivor whose patch git refused, which leaves
the bundle written and the exit code saying it is not the one that was asked for.
A run that tested nothing is `0` from `bundle` with no bundle written, because a
successful run with no evidence in it is not a failure of anybody's.

The code is authoritative. Where this page and `crates/tremula/src` disagree, the
code is right and this page is a bug.
