# Contracts

The files the tremula core and its language packs exchange. A pack and a core
that agree on these documents interoperate; nothing else crosses the boundary
except the command lines in [pack-protocol.md](pack-protocol.md).

## `schemas/`

JSON Schema (draft 2020-12) for every document: `manifest`, `spans`,
`capabilities`, `probe`, `baseline`, `results`, `report`, `pack-error`, `triage`,
`suppressions`, and `bundle`.

**These files are generated — do not edit them by hand.** They are produced from
the Rust types in `crates/tremula-contracts/src/`, which are the source of
truth, including their doc comments: each becomes the `description` of the field
it documents. Change a type, then regenerate:

```sh
just contracts
```

A test compares every committed schema against a freshly generated one, so a
hand edit or a forgotten regeneration fails the build rather than drifting.

## `examples/`

Fixtures shared by the Rust and Python test suites. Every valid example must
satisfy three checks at once: its schema accepts it, the model deserializes it,
and re-serializing reproduces every key and value it carries. That last one is a
comparison of documents rather than of bytes — the files are indented for reading
and a producer writes one compact line — so it catches a field dropped or renamed
in serialization, not a change of layout. Files named `invalid-*` exist to be
rejected, and are asserted to fail both schema validation and deserialization.

**Identifiers and hashes in these files are illustrative.** They are
well-formed — right length, right alphabet — but not actually derived from any
real content. In particular a mutant's `id` in an example is not the SHA-256 its
own fields would produce. The canonical derivation is specified in the `id`
field's description in `schemas/manifest.schema.json` and enforced by the core's
validation, not by these fixtures.

The exception is the `spans/` examples: they are the Python pack's own answer for
the files under `packs/python/tests/fixtures/spans_project/`, offsets and hashes
included. Two tests in that pack hold them to it. The document the subcommand
prints equals the example, and the example read back and written out again *is*
the line the subcommand printed, character for character — which is how the
layout the files are stored in stays a matter of reading them rather than a place
for serialization to drift unseen. Editing one by hand makes it wrong.

## Writing a producer

The compatibility rules a producer and consumer share — unknown fields,
extensible enums, and how optional fields are spelled — are specified once, in
the [contracts chapter](../docs/02-contracts/contracts.md#compatibility-and-evolution).

A mutant's `provenance` is free-form, but the keys a generator writes into it are
conventional. [pack-protocol.md](pack-protocol.md) says what they are.
