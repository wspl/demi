# Native utility dependencies

`packages/native-utils` owns the adapters that expose standard utilities to the
runner's shell. Their third-party sources live in the repository root's
`vendor/<crate>/`. Each source tree retains its upstream Cargo metadata, version
and license. The root `Cargo.toml` selects the patched crates through
`[patch.crates-io]`; they are not Demi workspace members.

The MIT-licensed `uucore`, `uucore_procs` and selected `uu_*` sources originate
from crates.io release 0.11.0. Other utility sources include `uu_grep`, `sed`,
`findutils`, `diffutils`, `jaq`, `jaq-std`, `ripgrep`, `ignore` and `grep-cli`;
their own Cargo manifests record their individual versions and licenses.
The interpreter is documented separately in
[Native shell dependency](native-shell-dependency.md).

The local changes give an invocation its own standard streams, utility name,
exit code and working directory. They allow native shell builtins to run in
parallel within a process. File and text algorithms remain in uutils.

Each invocation runs on a dedicated thread, which owns its uutils localization
state. A shell job owns those threads and its process lifetime. No command
may redirect the process-wide standard descriptors or change process cwd.

`packages/native-utils/tools/context-imports.py` adapts selected standard-library
imports in newly imported utility crates under `vendor/`. Runtime adapters stay
in `packages/native-utils/src/`; invocation-context implementations stay beside
the vendored code they adapt. Windows path handling uses the shared
`packages/native-path` crate.
