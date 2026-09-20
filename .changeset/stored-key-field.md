---
'@demicodes/web-ui': patch
---

`CommitTextInput` takes `stored`: a secret the page never receives shows as a filled, masked field, empties for typing when focused, and left empty keeps what is stored. The API key row uses it and drops its long description; the provider's Rename button is ghost.
