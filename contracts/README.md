# Contracts

The files the tremula core and its language packs exchange. A pack and a core
that agree on these documents interoperate; nothing else crosses the boundary
except the command lines in [pack-protocol.md](pack-protocol.md).

## `schemas/`

JSON Schema (draft 2020-12) for every document: `manifest`, `results`,
`baseline`, `report`, `capabilities`, `pack-error`, and `spans`.

**These files are generated — do not edit them by hand.** They are produced from
the Rust types in `crates/tremula-contracts/src/`, which are the source of
truth, including their doc comments: each becomes the `description` of the field
it documents. Change a type, then regenerate:

```
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

Three conventions are easy to get wrong:

- **Optional fields may be omitted or sent as `null`; both mean absent.**
  Producers also differ in whether they emit an empty optional map such as
  `provenance` or `backend_raw` as `{}` or leave the key out entirely. Both are
  valid, and consumers must treat them identically.
- **Ignore unknown fields.** Adding a field is a compatible change, so a
  consumer that rejects unknown keys will break on the next minor version.
- **Ignore unknown enum values, where the enum says to.** An enum whose value set
  includes `unknown` expects a consumer to read anything else as `unknown` rather
  than reject the document — that is what lets a value be added later, and the
  `stage` of a failure report and the `kind` of an excluded span both work this
  way. The schema of such an enum says `"type": "string"` and nothing more, so a
  consumer that validates before it parses accepts the same documents its parser
  does; the values this version defines are named in the enum's `description`
  instead, where they inform a reader without binding a later producer. An enum
  without an `unknown` value, such as `language`, does *not* work this way: its
  schema lists its values, and a consumer that meets one it does not know has to
  refuse, because an unknown language has no pack that can run it.

A mutant's `provenance` is free-form, but the keys a generator writes into it are
conventional. [pack-protocol.md](pack-protocol.md) says what they are.
