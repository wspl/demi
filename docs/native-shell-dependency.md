# Native shell dependency

`vendor/brush-core` contains brush-core 0.5.0 from upstream
commit `96a26d0c66cbc018a1517e9562944418fef5b272`. Its MIT license is retained
beside the source. The workspace patches this crate for both the runner and
brush-builtins so they use one interpreter implementation.

The runner owns these adaptations:

- Functions and compound commands that own a pipeline subshell start as tracked
  interpreter tasks. Their consumers start before the pipeline waits for
  completion. Blocking builtins execute outside Tokio's reactor threads.
- Here-documents and here-strings use anonymous temporary files. Preparing a
  redirection does not depend on pipe capacity or platform pipe-size limits.
  Closing the descriptor releases the temporary file.
- Windows shell paths use `demi-native-path`, the same resolver as file commands
  and native utilities. For example, `/c/project` and `C:/project` refer to the
  same directory for `cd`, redirection, and explicit executable paths. External
  program arguments are passed unchanged for the program to interpret.

`packages/runner/tests/tasks.rs` verifies function and compound pipelines,
command substitution, and expanded here-documents with output larger than an OS
pipe buffer. The runner's job process owns the interpreter tasks and file
descriptors; process-group or Job Object cancellation releases them together.

The native utilities use a separate invocation context. On Linux, each zero-copy
transfer owns its intermediate pipe. Concurrent `cat`, `head`, and other utility
calls cannot consume another invocation's intermediate bytes.
