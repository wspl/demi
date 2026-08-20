---
'@demicodes/host-local': patch
'@demicodes/agent': patch
---

Make checkpoint persistence atomic and serialized: `LocalHostStore.writeJson` now writes through a same-directory temp file + rename so concurrent writers (other processes included) and mid-write process death can no longer tear the stored JSON, and `AgentSession` serializes checkpoint writes so a boundary flush never overlaps a scheduled write and the last completed write always carries the newest snapshot.
