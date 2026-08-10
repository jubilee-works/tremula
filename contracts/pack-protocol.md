# Pack protocol

A tremula language pack is a program the core invokes as a subprocess. The core
never imports pack code, and packs never decide verdicts. Everything crossing
the boundary is either a documented command line or a JSON document validated
against a schema in `contracts/schemas/`.

## Invocation

The core resolves an interpreter or executable for the target project and calls
the pack with one subcommand. The Python pack is invoked as:

```
<python> -m tremula_python <subcommand> [options]
```

## Subcommands

### `--capabilities`

Print one `capabilities.schema.json` document to stdout and exit 0. The core
compares it against its own supported ranges and refuses to continue on a
mismatch. `validate_checks` names the checks this pack's `validate` performs, so
a newer core can tell whether the checks it wants are available.

`subcommands` lists the work subcommands below — `validate`, `run`, `collect`,
`spans`. `--capabilities` is not among them: it is the handshake that produces the
document, and every pack is required to answer it.

A core requires the subcommands it needs, not every subcommand the pack lists, so
a pack that grows one stays usable by a core that has never heard of it.

### `validate --manifest <path> --project <dir>`

Perform language-level validation only — the neutral checks against the bytes on
disk are the core's. Exit 0 when all mutants pass, 2 otherwise.

The checks a pack reports through `validate_checks`, as the Python pack
implements them:

- `compiles_in_file` — the target file, with the replacement put in the span's
  place, still compiles. A replacement is judged by the file it makes and not by
  itself, because reading it alone answers a different question: `return errors`
  is no module and compiles as nothing on its own, while being exactly right
  inside a function. The file that is checked is the file the run produces —
  the pack splices it the same way the backend's operator does, newline shaping
  included — so a mutant this check accepts is one the backend can really apply.
  It is a compile rather than a parse: a parse accepts a function with two
  parameters of the same name, and the interpreter does not.
- `single_statement` — it is one top-level statement, since it has to stand in
  for one node.
- `round_trips` — reinjecting the parsed replacement loses none of its text.
  This is losslessness after newline normalization, not byte-for-byte equality:
  a replacement is compared with its trailing newlines stripped, because the
  span it replaces normally stops before the line's newline and both spellings
  have to produce the same file. What the check does catch is text a parser
  attaches to a node's surroundings rather than to the node — trailing comments,
  leading comments, blank lines — which would silently vanish on injection.
- `span_matches_node` — the manifest's byte span lines up with a node the
  backend can match, so a mutant that produces no work item means a real adapter
  bug rather than a span that never had a chance. A compound statement's node
  ends after the newline that closes it, so a span aimed at one has to take that
  newline in; a span over a bare `if` or `while` header lines up with nothing,
  whatever else is right about it.
- `ast_equal` — the mutated file's syntax tree differs from the original's. A
  replacement that only regroups an expression makes a different file and the
  same program, and reporting such a mutant as survived would blame a test suite
  for a difference that is not there. This is the validator rule that keeps a
  syntactically identical mutation out of a report, and it applies to every
  manifest — one a person wrote as much as one a generator produced.

### `run --manifest <path> --project <dir> --out <run-dir> [--tests <path>] [--timeout <seconds>]`

Execute every mutant in the manifest and write `baseline.json`, `results.json`,
`refusals.json`, and execution logs into the run directory. Exit 0 when the run
completed, 2 on infrastructure failure.

`--out` is the run directory itself, not a parent: its last path segment is the
run identifier the documents carry. `--tests` may be repeated, and each value is
passed to the test runner as given; omitting it leaves the project's own default
collection in place. `--timeout` is a positive number of seconds and applies to
each run of the suite; omitted, it is derived from how long the baseline took. Verdicts are not the pack's business: an attempt that
produced no judgement is reported through `execution_status`, and an unusable
baseline through the error channel below.

A mutant a run cannot apply is left out of the run and reported, not allowed to
end it. Every mutant the manifest names still gets an entry in `results.json`, in
the manifest's order; a refused one carries `execution_status: "not_applied"` and
the reason under `backend_raw.refusal`, as a `code` and a `message`. The record the
entry is built from is `refusals.json`, written before anything is executed and read
back by `collect`, because nothing else in a run directory has anywhere to keep it:
the session holds jobs, and a refused mutant has none.

Only when *every* mutant is refused does the run fail through the error channel, at
whichever of the three points below empties the batch. There is nothing left to run
by then — no session at all, or one whose every job has been marked skipped — and no
results document worth writing either way.

Three things can refuse a mutant this way, and each is a mutant the manifest was
right about and the project cannot take:

- the `validate` checks above, applied per mutant instead of stopping at the
  first — including `ast_equal`, so a mutation with the original's syntax tree is
  now reported as never applied rather than executed and reported as survived;
- the session file the execution backend reads its parameters out of, when a
  mutant's own text does not survive being written to it;
- the backend's count of the jobs it made for one mutant, when that is not
  exactly one. None means the mutation could not be matched; more than one means
  no single match is the mutation described, so every one of them is left unrun.

`not_applied` is what a run reports rather than what it hides: the core's exit
precedence makes any `not_applied` entry exit 2, so a batch preserved this way is
still a run that says something went wrong.

### `collect --out <run-dir>`

Rebuild `results.json` from the backend state already present in the run
directory — the session, and the `refusals.json` a run leaves beside it — without
executing anything. Idempotent, and safe to call after an interrupted `run`. A run
directory with no `refusals.json` refused nothing a reader can name, which is also
true of every run directory made before that file existed.

### `spans --file <path> --project <dir>`

Report the functions of one file and the parts of them a mutation must not aim at.
Print one `spans.schema.json` document as the last line of stdout and exit 0, or
2 on failure. Nothing is written to disk and no run directory is involved: the
caller is deciding what to propose and has nothing to run yet.

`--file` is POSIX-style and relative to `--project`, spelled the way a manifest
spells the same file. Every offset in the answer is a raw byte offset into the
bytes `file_sha256` hashes, and every span is half-open — `[start_byte,
end_byte)`.

- `span` is the whole function, from the `def` or `async def` that opens it to the
  end of its last statement. Decorators are outside it, because they come before
  that keyword.
- `body_span` runs from the start of the first body statement to the end of the
  last, so the signature is outside it, and so are the comments and blank lines
  between the signature and the first statement. It is deliberately not "after the
  colon": a parser gives no position for the colon, and nothing between it and the
  first statement is a statement to mutate.
- `excluded` names the stretches of `body_span` that carry no behaviour — a
  `docstring`, as the whole statement rather than the string inside it, and the
  `annotation` of an annotated assignment. A parameter or return annotation needs
  no entry, being part of the signature and so outside `body_span` already. Absent
  means there are none.
- `qualified_name` is for a person to read and is never a key: overloaded
  definitions, a function defined twice under different conditions, and a
  property's getter and setter all report the same name. `span` is what identifies
  a function.

This call is made directly, with no manifest, so none of the neutral validation
the core performs before a run has happened. The pack therefore reads the file on
its own terms and refuses, through the error channel below, a file whose bytes are
not UTF-8, one that begins with a byte-order mark, one whose leading comments
declare an encoding other than UTF-8, one that holds a carriage return, and a path
that is not a plain project-relative POSIX path, lands outside the project, or
reaches the file through a link — the file itself or a directory on the way to it.

Every one of those is a file the core would refuse a run over, and a pack draws
the line where the core draws it rather than where it would prefer to. Spans over
a file no run could use are offsets nobody can act on; spans withheld from a file
a run would have accepted put targets out of reach for no reason the caller can
see.

#### What a function owns

A function's mutation targets are its `body_span` minus the `span` of every
function nested inside it. Every function of the file is in the same report, so a
consumer derives that without looking at the source again.

Worked over `examples/spans/async_nested.json`, whose file is 520 bytes:

| function | `span` | `body_span` | `excluded` |
| --- | --- | --- | --- |
| `sync_events` | `[96, 519)` | `[188, 519)` | docstring `[188, 231)` |
| `sync_events.<locals>.merge` | `[237, 459)` | `[353, 459)` | annotation `[361, 375)` |

`sync_events` owns `[188, 519)` less `[237, 459)`, which is `[188, 237)` together
with `[459, 519)`. Its own docstring takes `[188, 231)` out of the first, leaving
`[231, 237)` — the newline, the blank line, and the indent before `def merge` —
and `[459, 519)`, which is the `await` and the `return` after the nested
definition. Everything `merge` is made of belongs to `merge`, whose own entry
excludes the `dict[str, str]` of its annotated assignment.

One thing the subtraction does not remove: a nested function's decorators sit
between the parent's statements and the child's `def`, so they stay inside the
parent's area, and no span in the report covers them. That is said plainly rather
than papered over. A mutation aimed at a decorator is a mutation aimed at a
header, and a header inside a body is not something this document can fence off —
what catches it is the same check that catches every other one: confirming that
what came back changes the function it claimed to change.

## Errors

On failure a pack prints one `pack-error.schema.json` document as the last line
of stdout and exits 2:

```json
{
  "error": {
    "stage": "baseline",
    "code": "baseline_failed",
    "message": "3 tests failed before any mutation was applied"
  }
}
```

`stage` is one of `preflight`, `spans`, `validate`, `baseline`, `plan`, `execute`,
`collect`, and tells the core how far the pack got. `code` is a stable
machine-readable identifier; `message` is for humans and carries no contract.

A consumer that meets a stage it does not know reads it as `unknown` rather than
refusing the report: what went wrong is in `code` and `message`, and losing those
to a step named after the consumer was built would be the worse trade. The schema
holds `stage` to being a string and no more, so validating a report before parsing
it does not undo that tolerance; the seven names above are recorded in the field's
description rather than as values a validator enforces.

### Failure codes

Every code the Python pack reports, by the stage that reports it. A code is stable
in the sense `message` is not: the wording may be rewritten, the identifier is what
a reader may branch on, so it is listed here or it is not one.

A pack may report a code this list does not name — a consumer that met one would
read the message and treat the failure as diagnosed but unrecognised — but this
pack does not, and its own suite holds it to that.

Preflight, before any mutant is touched:

- `invalid_arguments` — the command line is not one this pack can act on.
- `invalid_run_directory` — `--out` has no last path segment to use as a run
  identifier.
- `operator_not_registered` — Cosmic Ray cannot resolve this pack's operator, which
  usually means the two are installed in different environments.
- `unsupported_cosmic_ray` — the installed backend is outside the pinned range.

Validation of the manifest's language rules:

- `invalid_manifest` — the document is not a manifest this pack can read.
- `replacement_does_not_compile` — the target file does not compile with the
  replacement in the span's place. The `compiles_in_file` check above.
- `invalid_replacement` — the replacement cannot stand in for a single node:
  `single_statement` or `round_trips`.
- `span_matches_no_node` — the span is not exactly one node the backend can match.
- `mutation_is_ast_equal` — the mutated file has the original's syntax tree.

The reference run, the session, and the execution:

- `baseline_failed` — the suite did not pass before any mutation was applied, so
  nothing after it would mean anything.
- `unserializable_mutant` — the mutant's own text does not survive being written to
  the session file the backend reads its parameters out of.
- `mutant_job_mismatch` — the backend made no job for a mutant, or more than one.
- `init_failed` — the backend could not build a session from the generated
  configuration.
- `exec_failed` — the backend stopped before it finished running the session. The
  session survives, so `collect` still reports what did run.
- `every_mutant_refused` — every mutant in the manifest was refused, at whichever of
  the three points above emptied the batch.

Rebuilding a run's results:

- `incomplete_run_dir` — the directory is missing something a run writes before it
  executes anything, so it is not a run directory to collect.

Reporting where a mutation may land, all of which are about the file `spans` was
asked for:

- `invalid_target_path` — not a project-relative POSIX path.
- `target_reached_through_link` — the file, or a directory on the way to it, is a
  link.
- `target_outside_project` — the path lands outside the project.
- `target_missing` — there is no such file in the project.
- `target_unreadable` — there is, and it could not be read.
- `target_has_byte_order_mark` — it begins with a byte-order mark.
- `target_not_utf8` — its bytes are not UTF-8.
- `target_declares_other_encoding` — its leading comments declare another encoding.
- `target_has_carriage_return` — it uses carriage returns.
- `target_does_not_parse` — it is not valid Python.
- `project_root_unresolvable` — `--project` does not resolve, or is not a directory.
- `unpositioned_node` — the parser reported a node with no position, so no span of
  this file can be trusted. A defect in the pack, reported as one.

And two that belong to no step, because either can end any of them:

- `interrupted` — the pack was stopped from outside before it finished. The run
  directory keeps whatever the run had already done, and `collect` can report it.
- `unexpected_error` — a failure the pack did not plan for, carrying the exception's
  type and text rather than a traceback. Its `stage` is how far the pack had got.

## Output discipline

Diagnostics go to stdout, because an execution backend may discard stderr. The
last line of stdout is reserved for machine-readable output: the capabilities
document, an error object, or nothing.

## Provenance

A mutant's `provenance` is free-form and excluded from its identifier, so what a
generator records there can change — or be recorded twice differently — without
changing what the mutation is. Free-form in the schema, conventional in practice:
a generator says what produced the mutant under the `generator` key.

```json
{
  "generator": {
    "name": "tremula-generate",
    "version": "0.1.0",
    "model": "example-model-2026-05-01",
    "prompt_version": "3",
    "generated_at": "2026-08-10T09:12:44Z"
  }
}
```

- `name` and `version` — the program that produced the mutant and its release, the
  same pair a pack reports about itself.
- `model` — the model that answered, spelled the way its provider names that exact
  snapshot. A family name is not enough: it names whichever version is current,
  which is the one thing a reproduction cannot rely on.
- `prompt_version` — which revision of the prompt was used, so a change in what
  the mutants look like can be traced to a change in what was asked.
- `generated_at` — when the answer came back, as RFC 3339 in UTC.

None of it is enforced. The schema leaves `provenance` open and consumers ignore
what they do not recognise, which is what lets a generator record more than this —
token counts, a request identifier, whatever its own operators need. The convention
exists so that two producers recording the same fact record it under the same name.
A producer with nothing to say leaves `provenance` out; an empty object means the
same.

## Compatibility

Documents carry `schema_version`; capability and pack metadata report the same
value as `contract_version`. Consumers ignore unknown fields, so new fields are
compatible.

New enum values are compatible where the enum defines an `unknown` value: a
failure's `stage` and an excluded span's `kind` both do, and a consumer reads
anything it does not recognise as `unknown`. Their schemas are written to match,
constraining such a value to a string and listing this version's values in the
description only — a schema that enumerated them would have a validator reject the
documents the fallback exists to keep readable. Where an enum defines no such
value — `language`, which selects the pack — its schema does enumerate them, and a
new value breaks older consumers on purpose, because an unknown language has no
pack that could run it.

That fallback is what a consumer owes the document, not a licence for a producer: a
pack still emits only the values its declared contract version defines.
