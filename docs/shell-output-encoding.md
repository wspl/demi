# Shell output encoding

Shell transport carries bytes. Registered command IO accepts Unicode strings or
Uint8Array values: strings are encoded once as UTF-8, and bytes remain unchanged.
The just-bash interpreter aggregates statement/script and compound-command
output as explicitly marked byte strings. Pipes, redirections, functions and
control-flow exits retain this shape. Both stdout and stderr carry explicit kinds;
no layer guesses their encoding from the characters in the string.

The public Bash API converts valid UTF-8 to its text view and retains invalid
byte results with their byte kind. Demi shell consumes the interpreter byte
contract directly, strictly decodes for display, and stores invalid stdout and
stderr unchanged in stdout.bin/stderr.bin with a placeholder in the text view.
UTF-8 BOMs are content and remain intact. Live process/registered-command views
use persistent decoders across chunks; display offsets count emitted UTF-8 bytes,
not the size of the most recent source fragment. Raw capture limits are separate.
Background text views also carry decoder state until stream completion.

Ownership stays within existing package boundaries: just-bash owns interpreter
semantics, shell owns Host capture and artifacts, utils owns portable codecs, and
agent consumes shell results without another decoding pass.

## Verification

- `packages/shell/src/__tests__/output-encoding.test.ts`: Latin-only text, Unicode,
  pipelines, groups, functions, loops, raw binary, exit, mixed-output redirection,
  descriptor duplication, and one-byte chunks on stdout/stderr.
- `packages/shell/src/__tests__/binary-stdout.test.ts`: raw artifacts, byte limits,
  and actual external-process input/output round trips.
- `packages/just-bash/packages/just-bash/src/encoding-lossless-output.test.ts`:
  interpreter compound/exit/eval paths, mixed streams, and split UTF-8 bytes across
  statements. Existing encoding, syntax and interpreter tests guard text behavior.

External-process and GNU-bash oracle suites require Linux. macOS LocalHost's
existing /dev/fd directory anchoring can return ENOTDIR and is outside this change.
