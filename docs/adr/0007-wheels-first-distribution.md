# 7. Wheels are the first distribution channel

## Status

Accepted

## Implementation note (2026-08-25)

A release carries the wheels themselves alongside the extracted binaries and
checksums, so that an internal consumer can install tremula from a private
release before anything is on PyPI.

## Context

[ADR-0002](0002-rust-core-with-language-packs.md) settled the shape of the
artifact: the Rust core is packaged as a wheel, so installing tremula is
installing a Python package and the user never learns that a compiler was
involved. It left the surrounding questions open. Where do the wheels live?
How does a user in CI get one? What is a GitHub Releases binary for, if
anything?

Two pressures converge here. On one side, Python developers install Python
packages, and every step that is not `pip install` costs users. On the other
side, a Rust binary in a wheel tempts workarounds: shell scripts that curl an
archive from a release page, checksums verified by hand, installs that build
from source when no wheel matches. Each workaround is a supply-chain question
and a support burden, and none of them carries the lockstep between the core
and its language pack.

The lockstep itself is already built. The `tremula` wheel declares
`Requires-Dist: tremula-python==0.1.*`, so the wheel carries the version
pairing that ADR-0002 promised, and pip resolves both packages from one
command.

## Decision

Distribution has three tiers, in order of importance.

1. **PyPI wheels are the default install path.** The `tremula` wheel declares
   `tremula-python==0.1.*` as a dependency, so installing tremula pulls the
   matching language pack with it. The lockstep rides on the wheel itself, not
   on documentation asking users to install two things together.
2. **A composite GitHub Action encapsulates the usage path.** Installation
   inside it is the single line `pip install tremula==<version>`; the Action's
   value is exit-code handling, artifact upload, and comment wiring. The
   curl-based install path is retired: it was exposed to API rate limits, it
   verified checksums with hand-rolled scripting, and it could not receive the
   Trusted Publisher attestation that the PyPI path gets for free.
3. **GitHub Releases binaries are a by-product plus checksums.** The Python
   pack is the only pack, so every current user is a Python user and nobody
   needs a bare binary. The binaries stay attached to releases because they
   cost nothing, and they are promoted to first-class when a non-Python pack
   exists.

Two platforms ship wheels: `linux-x86_64` and `macOS-arm64`. Those cover CI
and the team's laptops.

**No sdist is published.** On an unsupported platform, failing clearly with
"no matching distribution found" beats failing slowly through an attempted
Rust-toolchain source build. A clear failure tells the user what to do; a slow
one tells them nothing after ten minutes.

**Windows is deferred, and the deferral has a precondition.** The pack path
convention assumes `bin/python` and `bin/cosmic-ray`, which is POSIX. Windows
wheels need a `Scripts/` branch in that convention first; shipping a wheel
before that branch exists would ship a broken install.

This record **extends ADR-0002; it does not supersede it.** The original
decision, that binaries are packaged as wheels, stands. What this record adds
is where the wheels live and what the Action and Releases tiers are for.

## Consequences

Installing tremula, once the packages are published, is one pip command on the
two supported platforms, and the pack arrives with the core because the
dependency says so. The version pairing stops being a convention and becomes
something pip enforces.

Nothing in this record publishes anything. The structure is decided; the
timing of the first PyPI upload is gated on decisions recorded elsewhere (D1
through D3 in the distribution decision: trademark review, placeholder
publish timing, and account and Trusted Publisher ownership). Until that gate
opens, installing from source remains the way to get tremula.

Users on platforms without a wheel get a fast, explicit failure rather than a
slow, confusing one. That is a feature, but it is also a commitment: a new
platform is a deliberate decision to build and support a wheel, not an
accident of a source build happening to work.

Windows support has a named prerequisite, the `Scripts/` path branch, tracked
as its own issue. Until that lands, the deferral is a decision, not an
oversight.

The Releases binaries exist with checksums and no consumers. If a non-Python
pack ever ships, this record already says what changes: the tier promotes, and
the install story for that pack is written then.
