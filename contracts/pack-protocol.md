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

- `parses` — the replacement is syntactically valid code.
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
  bug rather than a span that never had a chance.

### `run --manifest <path> --project <dir> --out <run-dir> [--tests <path>] [--timeout <seconds>]`

Execute every mutant in the manifest and write `baseline.json`, `results.json`,
and execution logs into the run directory. Exit 0 when the run completed, 2 on
infrastructure failure.

`--out` is the run directory itself, not a parent: its last path segment is the
run identifier the documents carry. `--tests` may be repeated, and each value is
passed to the test runner as given; omitting it leaves the project's own default
collection in place. `--timeout` is a positive number of seconds and applies to
each run of the suite; omitted, it is derived from how long the baseline took. Verdicts are not the pack's business: an attempt that
produced no judgement is reported through `execution_status`, and an unusable
baseline through the error channel below.

### `collect --out <run-dir>`

Rebuild `results.json` from the backend state already present in the run
directory, without executing anything. Idempotent, and safe to call after an
interrupted `run`.

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
