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

## The order of a run

The sequence below is mostly forced rather than chosen; each step is either the
reason the next one is safe or the reason it means anything.

1. **Read and parse the manifest.** Its declared `schema_version` has to be one
   this binary reads. Nothing in the project has been touched yet, so nothing can
   have raced.
2. **Claim the project.** `.tremula/lock` records the run's identifier and its
   process id, created exclusively so that two runs cannot both hold it. Mutants
   are applied in place and a project has one source tree, so a second concurrent
   run would corrupt the first one's files. A lock whose process is gone is taken
   over, with a warning; a lock whose process cannot be signalled counts as held,
   because that is what a process belonging to another user looks like.
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
   `.venv`. It has to have the pack installed, and the pack has to report a
   contract version this binary speaks, the subcommands a run uses, and at least
   one check of its own.
7. **Reserve the run directory, write the snapshot, publish it as `latest`.** The
   snapshot is the bytes from step 5 rather than a second reading of the files.
   Publishing before the pack starts is deliberate: the run a reader needs
   `tremula restore` to find is precisely the one that did not finish.
8. **Run the pack.** It performs its own preflight, its language checks, the
   reference run, then applies every mutant and collects the results.
9. **Read back what it wrote, and check that it belongs.** Both documents have to
   carry this contract version, this run's identifier, and this pack's name and
   version. A pack that reports success without leaving a document is a defect in
   the pack, and is reported as one.
10. **Judge, write `report.json`, print.** The last line of stdout is
    `run_dir=<path>`, absolute, and the process exits with the report's own exit
    code.

## What a run leaves behind

```
.tremula/
├── lock                       # while a run holds the project
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

A run directory is reserved, never reused: two runs of the same manifest within
one second would otherwise interleave their documents, so the second gets a name
of its own.

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
