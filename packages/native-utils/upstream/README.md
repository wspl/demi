# Contextual uutils sources

These MIT-licensed sources originate from the crates.io releases of
`uucore` and the selected `uu_*` crates at version 0.11.0. Each source tree
retains its upstream license and package metadata.

The local changes give an invocation its own standard streams, utility name,
exit code and working directory. They allow native shell builtins to run in
parallel within a process. File and text algorithms remain in uutils.

Each invocation runs on a dedicated thread, which owns its uutils localization
state. A shell job owns those threads and its process lifetime. No command
may redirect the process-wide standard descriptors or change process cwd.
