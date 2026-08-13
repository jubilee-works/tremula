# The run lifecycle

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

```text
.tremula/
├── lock                       # while a run holds the project
├── lock.gate                  # whose turn it is to take an abandoned lock over
└── runs/
    ├── latest -> <run-id>
    └── <run-id>/              # UTC timestamp + the first six hex digits of the manifest's hash
        ├── manifest.json      # the input, copied by the pack
        ├── config.toml        # the backend configuration the pack generated
        ├── session.sqlite     # the backend's own state
        ├── targets.json       # the manifest targets selected for this run
        ├── expected-hashes.json # target hashes used to detect drift
        ├── diffs.json         # per-mutant patches recovered from the backend
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

Manifest parsing, locking, and validation happen before a run directory is
reserved. A failure there leaves neither a report nor a run directory. Only some
files exist for a failure after reservation: a reference-run failure has neither
the pack's `manifest.json` copy nor `refusals.json`, because both are written when
the pack plans the session. That is normal, not damage. `triage.json` exists only
for a run somebody has triaged, and a run whose `manifest.json` or `snapshot/` is
missing cannot be triaged at all: those two are where a survivor's mutation and
the bytes it was measured against come from.

The derived-manifest mode never changes the source manifest, report, or snapshot.
Its optional paired `--exclude-suspected-equivalent --out-manifest PATH` mode
writes a new manifest at the user-named path, after triage writes `triage.json`
as usual. That next-run input removes only non-dismissed survivors the model marked `suspected_equivalent`; it doesn't record a suppression.

The project's dismissals are not in here. `tremula-suppressions.json` sits beside
the manifest, is meant to be committed, and outlives every run — which is the point
of it.
