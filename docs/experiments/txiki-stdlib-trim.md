# Embedded runner standard-library trimming experiment

Measured on 2026-09-08 against Demi `aa8f7c50` and its pinned txiki.js
fork `1a4c5b01`. The experiment was reverted: removing the unused public
modules and CLI/REPL saved approximately 110 KiB, or 3–4% of the complete
runner. This did not justify maintaining a separate runtime configuration.

## Scope and possible design

Neither the runner nor the current Demi command modules directly imports
`tjs:*`. The development `tjs` CLI does: its command dispatcher uses
`tjs:getopts` and `tjs:path`, and its REPL uses `tjs:readline` and `tjs:utils`.

A production implementation would keep the development runtime unchanged
and build a separate embedded runtime without public standard-library
registration, the CLI dispatcher, or the REPL. The existing `tjs` global,
Web APIs, module loader, and internal modules would remain available.
In particular, `console` still needs its bundled formatting implementation,
and runtime initialization needs `tjs:internal/path`.

## Method

The temporary native-source patch removed all public entries and their
bytecode includes from `src/builtins.c`, removed the REPL binding from
`src/mod_sys.c`, and removed the non-embedded dispatcher reference from
`src/vm.c`. Existing linker dead-code stripping then omitted CLI/REPL
bytecode. No JavaScript bundle or compiler settings changed.

This removed nine public modules: `assert`, `getopts`, `hashing`, `ipaddr`,
`path`, `posix-socket`, `readline`, `utils`, and `uuid`. FFI, SQLite, and
WASM were already disabled. The experiment retained shared native
implementations; it did not attempt to remove every native function behind
these modules.

## Results

| Complete application | Baseline bytes | Trimmed bytes | Saved bytes | Reduction |
|---|---:|---:|---:|---:|
| macOS ARM64 | 3,215,936 | 3,100,800 | 115,136 | 3.58% |
| Linux ARM64 musl | 3,468,952 | 3,358,240 | 110,712 | 3.19% |

The trimmed macOS application passed the existing command-mode fixture
using the experimental binary: file creation and reading, help output,
expected standalone RPC rejection, unknown command handling, and runner
usage output (one test, 12 assertions). Its strict code-signature check
also passed. The trimmed Linux application was cross-built but not run.
The full runner suite and KVM tests were not repeated for this experiment.

All three native source edits were restored afterward. The rebuilt Linux
application matched its saved baseline byte for byte. The rebuilt macOS
application had the same size and matched after excluding its generated
Mach-O UUID and code signature; strict signature verification passed.
The fork and production build configuration remain unchanged.
