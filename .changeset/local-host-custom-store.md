---
'@demicodes/host-local': patch
---

`LocalHostOptions.store` lets embedders bring their own `HostStore` (e.g. to wrap or gate persistence writes); `LocalHostStore` is now exported for composition. Defaults are unchanged.
