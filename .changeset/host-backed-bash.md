---
'@demicodes/shell': minor
'@demicodes/host-local': minor
---

Host-backed shells now match GNU bash on the observation surfaces models use.

Unix names (`ls`, `grep`, `whoami`, …) spawn the PATH binary first; the portable implementation runs only when spawn reports `executable_not_found`. `Host` gains `identity` and `process.openCwd` (a directory fd on Linux) so a deleted cwd is not classified as a missing binary, children receive the exported env only, and `type` / `$UID` / `test -O` describe the Host principal. Custom Hosts must implement the new fields.
