---
"@demicodes/machines": patch
---

A Cloud machine's disks cost what its guest writes: the manager's state belongs on a filesystem that clones files, the development instance now keeps it on one, and the manager says at startup when it is on a filesystem that copies instead.
