# Work package <id>: <one-line purpose>

<!-- The tech lead fills every section. Delete nothing; write "none" instead. -->

## What to build

<What the package owns, in two or three sentences, and the checkpoint this
run delivers: the API checkpoint or the implementation checkpoint.>

## Read first

- The design: <docs/... sections>
- The Rust source to port: <crates/...>, and its tests: <crates/.../tests>
- The package contract: `docs/architecture/crates-and-packages.md` § <entry>

## Write boundary

Write only inside these paths; `scripts/gomig/boundary.sh` refuses a branch that changed anything else:

- `<internal/...>`

## API

<The exported identifiers this package must provide, with their doc
comments' first sentence, or "as the API checkpoint merged in <commit>
defines; do not change it".>

## Dependencies

- Demi packages you may import: <list>
- Modules you may import: <list from go.mod>. Need another? Stop and say so in
  your report; do not edit `go.mod`.

## Tests

<The Rust tests to port, the scenarios to write, the fixtures to use, the
build tags (`hostonly` for Chrome, FSEvents or process listing).>

## Done when

Run from the repository root, all must pass:

    scripts/gomig/check.sh <package patterns>

<Any further check, such as a corpus or a generated-TypeScript comparison.>

## Report

Commit your work on your branch, then write the report to
`/Users/zan/Projects/demi-worktrees/gomig-ref/reports/<id>.md` from
`scripts/gomig/report-template.md`. The report is not part of the repository.
