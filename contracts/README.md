# Contracts

The files the tremula core and its language packs exchange. A pack and a core
that agree on these documents interoperate; nothing else crosses the boundary
except the command lines in [pack-protocol.md](pack-protocol.md).

## `schemas/`

JSON Schema (draft 2020-12) for every document: `manifest`, `results`,
`baseline`, `report`, `capabilities`, and `pack-error`.

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
and re-serializing reproduces it byte for byte. Files named `invalid-*` exist to
be rejected, and are asserted to fail both schema validation and deserialization.

**Identifiers and hashes in these files are illustrative.** They are
well-formed — right length, right alphabet — but not actually derived from any
real content. In particular a mutant's `id` in an example is not the SHA-256 its
own fields would produce. The canonical derivation is specified in the `id`
field's description in `schemas/manifest.schema.json` and enforced by the core's
validation, not by these fixtures.

## Writing a producer

Two conventions are easy to get wrong:

- **Optional fields may be omitted or sent as `null`; both mean absent.**
  Producers also differ in whether they emit an empty optional map such as
  `provenance` or `backend_raw` as `{}` or leave the key out entirely. Both are
  valid, and consumers must treat them identically.
- **Ignore unknown fields.** Adding a field is a compatible change, so a
  consumer that rejects unknown keys will break on the next minor version.
  Adding a value to an existing enum is *not* compatible: a consumer that does
  not know the value will reject the document.
