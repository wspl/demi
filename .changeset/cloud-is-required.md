---
'@demicodes/backend': minor
'@demicodes/web-ui': minor
'@demicodes/web': patch
---

Every deployment has Cloud. The backend requires a provisioner
(`managedHosts`) and its executable refuses to start without
`DEMI_MACHINES_SOCKET` and `DEMI_BACKEND_PUBLIC_URL`; the `no_cloud` answers and
the product state's nullable `cloud` are gone, and the workspace dialog always
offers Cloud.
