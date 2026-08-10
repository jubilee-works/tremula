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

## What each side owns

| | Core (`crates/tremula`) | Pack (`packs/python`) |
| --- | --- | --- |
| Reads | manifest, `results`, `baseline` | manifest |
| Writes | `report`, the console report | `results`, `baseline`, execution logs |
| Owns | validation against bytes on disk, the project lock, run directories, snapshots, verdicts, the exit code | the environment check, language-level validation, the reference run, applying mutants, running the suite |
| Never | starts a test suite, parses source, names a backend | decides a verdict, decides an exit code beyond "worked" or "failed" |

The pack is invoked as `<python> -I -m tremula_python <subcommand>`. Running it as
an installed module rather than a script, through an isolated interpreter, is
what makes the pack the core negotiated with the pack that actually runs: a
directory named `tremula_python` in whatever the user was standing in cannot
answer for it, and neither can `PYTHONPATH`. Every path argument is absolute,
because the pack starts subprocesses with working directories of their own.

## Where mutants come from

A manifest has to be written by something. That something is a `MutantGenerator`:
one function in, one set of proposed mutations out, with the model that answered
and what the call cost. It is a library seam — no subcommand calls it yet, and
the order of a run below begins with a manifest that already exists.

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
   describes.
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
        ├── report.json        # verdicts, score, exit code — written by the core
        ├── snapshot/          # the target files as they were
        ├── pycache/           # bytecode, kept out of the project's own
        └── logs/              # what the pack and the suite said
```

A run directory is reserved, never reused: runs of the same manifest within one
second would otherwise interleave their documents, so each after the first gets a
suffixed name of its own, up to `-9`.

Only some of these exist for a run that failed early. A run that stopped before
its manifest was validated has a report and nothing else; one that stopped in the
pack's reference run has no `manifest.json` copy, because that copy is written
when the pack plans the session. That is normal, not damage.

## Exit codes

`0` — no mutant survived. `1` — at least one did. `2` — the run cannot be
trusted.

The third is the interesting one, and it is an explicit list rather than a
judgement call: a run-level failure (a lock conflict, a failing reference run, a
pack that could not start), a mutation that was never applied, a mutant that was
scheduled and never ran, results that do not cover the manifest exactly once, or
a manifest with mutants that produced no usable verdict at all. Anything else is
not `2`. One mutant with an environment problem is reported, excluded from the
score, and does not fail the run.

The code is authoritative. Where this page and `crates/tremula/src` disagree, the
code is right and this page is a bug.
