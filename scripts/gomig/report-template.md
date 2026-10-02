# Report: work package <id>

## Delivered

<What now exists, by package, in a few sentences.>

## Fidelity

Three tables; the tech lead checks each line against the Rust code.

1. Every Rust test (`scripts/gomig/rusttests.sh <crate>` lists them) and the
   Go test that ports it, or why it is not ported.
2. Every public Rust item of the crate and the Go identifier that replaces
   it, or why none does.
3. Every Go behavior with no Rust counterpart: a check, limit, default,
   retry, fallback or error path the Rust code does not have. "None" if none.

## Deviations

<Every place where the Go behavior differs from the design documents or the
Rust tests, with why. "None" if none.>

## Requests

<API changes needed in packages you do not own, modules to add to go.mod,
design gaps found. "None" if none.>

## Checks

<The commands you ran and the last lines of their output. Name any test that
needs `hostonly` and was not run.>

## Open issues

<What is unfinished or unverified.>
