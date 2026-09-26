# Brief T1a: text utilities

Slot s3 (/home/user/demi-slots/s3), branch go/wp/T1a-text. Rules: _rules.md.

Utilities: cat head tail wc tee sort uniq cut tr, GNU-compatible (coreutils 9.4
behavior with LC_ALL=C; black-box reference only).
Read first: docs/demi-next/runner.md § Shell jobs, internal/toolctx (the only
way to the outside), internal/toolctx/toolctxtest (tests), .golangci.yml,
the plan's Gate B criteria. Reference catalog and help routing:
/home/user/demi-base/crates/runner/src/shell/utilities.rs and
crates/runner/tests/utilities_{cat,catalog}.rs.

Deliver internal/tools/text: `var Utilities = map[string]toolctx.Utility{...}`
with the nine utilities. Requirements: every option of GNU coreutils 9.4 that
the T0 corpus marks core; `--help` prints usage to stdout and exits 0; paths
resolve through inv.Files; every loop checks inv.Context; `tail -f` follows a
file and ends at cancellation; `tee` and `sort -o` write through inv.Files;
errors are printed as the GNU tool prints them (program name prefix) with the
same exit status. You may fork permissively licensed code (for example u-root
BSD-3) under third_party/ if it helps; rule 5 applies.
Tests: the T0 corpora (testdata/gnu/<utility>/cases.json via
internal/testing/gnucorpus) once T0 has merged; until then, write your own
behavior tests, comparing against the GNU binaries in the test only through
recorded expectations, never by calling GNU tools at test time. Plus
cancellation tests (goleak) and --help tests.
Owned paths: internal/tools/text/**, any third_party fork you add, go.mod/go.sum.
