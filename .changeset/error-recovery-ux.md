---
'@demicodes/web-ui': patch
---

Single, live recovery entry point after errors. Continue now renders only on
the conversation's tail error/abort block while the session is idle — older
error blocks become records instead of accumulating competing Continue
buttons. Clicking Continue flips the phase to running optimistically, so the
loading state appears the moment the user acts instead of after the server
round-trip (rolled back if the resume call fails).
