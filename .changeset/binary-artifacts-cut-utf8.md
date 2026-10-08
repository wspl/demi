---
"@demicodes/agent": patch
"@demicodes/shell": patch
"@demicodes/utils": patch
---

Keep a command's artifacts when its stdout or stderr is binary, so the `stdout.bin` / `stderr.bin` path the result points to still exists. Output that is valid UTF-8 except for a character cut at the end (as by `head -c`) is now shown as text with a note instead of being treated as binary.
