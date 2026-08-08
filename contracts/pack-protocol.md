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

`subcommands` lists the work subcommands below — `validate`, `run`, `collect`.
`--capabilities` is not among them: it is the handshake that produces the
document, and every pack is required to answer it.

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
infrastructure failure. Verdicts are not the pack's business: an attempt that
produced no judgement is reported through `execution_status`, and an unusable
baseline through the error channel below.

### `collect --out <run-dir>`

Rebuild `results.json` from the backend state already present in the run
directory, without executing anything. Idempotent, and safe to call after an
interrupted `run`.

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

`stage` is one of `preflight`, `validate`, `baseline`, `plan`, `execute`,
`collect`, and tells the core how far the pack got. `code` is a stable
machine-readable identifier; `message` is for humans and carries no contract.

## Output discipline

Diagnostics go to stdout, because an execution backend may discard stderr. The
last line of stdout is reserved for machine-readable output: the capabilities
document, an error object, or nothing.

## Compatibility

Documents carry `schema_version`; capability and pack metadata report the same
value as `contract_version`. Consumers ignore unknown fields, so new fields are
compatible. New enum values are not: a pack must not emit a value the declared
contract version does not define.
