---
'@demicodes/command-protocol': minor
'@demicodes/runner-protocol': minor
'@demicodes/backend': patch
---

A native package descriptor and a runner manifest name the targets they carry instead of always all six, so a development release can hold only the targets in use. Publication still requires every target.
