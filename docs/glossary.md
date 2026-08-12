# Glossary

These terms keep the same meaning throughout tremula's user guides, architecture
notes, and contracts.

| Term | Meaning |
| --- | --- |
| **Core** | The Rust `tremula` binary. It validates manifests, coordinates work, decides verdicts, and writes reports and bundles. |
| **Language pack** | A subprocess that owns language-specific parsing, mutation application, and test execution. |
| **Backend** | The execution engine used inside a language pack. The Python pack uses Cosmic Ray. |
| **Manifest** | The input document that identifies each mutation by file, byte span, original text, and replacement text. |
| **Mutant** | One proposed source change in a manifest. |
| **Mutation** | The change a mutant describes: replace one span of source bytes with new text. |
| **Run** | One execution of a manifest against a project. A run has its own immutable identifier and working directory. |
| **Baseline** | The test suite result before any mutant is applied. Mutant results are compared with it. |
| **Execution** | One attempt to apply and test a mutant. This is narrower than a run, which contains the baseline and every execution. |
| **Verdict** | The core's judgement of an execution, such as `killed`, `survived`, or `runtime_error`. |
| **Killed** | The suite reacted to the mutation with a test failure, test error, or timeout. |
| **Survived** | The selected tests passed with the mutation applied. This does not prove that the mutation is reachable or meaningful. |
| **Refusal** | A reason the pack could not apply a mutant. Refused mutants are reported as `not_applied`. |
| **Triage** | The follow-up step that tries to distinguish a survivor by running one model-proposed input against both versions of its function. |
| **Dismissal** | A person's recorded decision not to raise the same mutation again. Dismissals live in `tremula-suppressions.json`. |
| **Run directory** | The working record under `.tremula/runs/`, including snapshots, backend state, logs, and contract documents. |
| **Bundle** | A checked, portable subset of a run directory intended for another person or coding agent. |
| **Contract** | A versioned JSON document or subprocess rule shared by the core, language packs, and evidence consumers. |

## Similar terms that are not interchangeable

- A **survivor** is an execution result. It is not automatically a missing test.
- A **refusal** means the mutation was not applied. It is not a verdict.
- A **run directory** contains working state and source snapshots. A **bundle**
  contains selected evidence for transfer.
- The **core** decides verdicts. A **pack** reports neutral execution signals.

See [Contracts](02-contracts/contracts.md) for the document model and
[Architecture overview](01-architecture/overview.md) for the core-pack boundary.
