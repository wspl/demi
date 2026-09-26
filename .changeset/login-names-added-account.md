---
'@demicodes/web-ui': patch
---

A completed subscription login reports the credential it added (`credentialId`), and the sign-in dialog names that account instead of the active one. Its `done` phase carries `active`: a first account reads as now active, one added beside another says the provider keeps its active account until this one is activated.
