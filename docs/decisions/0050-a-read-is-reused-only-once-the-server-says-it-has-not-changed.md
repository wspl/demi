---
status: accepted
---

# A read is reused only once the server says it has not changed

## Context and Problem Statement

The page reads a conversation's settings, a file's text or its changes
again and again. Some of those reads sent an ETag of their own and handled
"not changed" in the page, each in its own way; most sent the whole answer
every time, and the page told the browser never to keep any.

## Considered Options

* Extend the page's own revalidation to every read.
* Let the server tag every read and the browser revalidate it, as HTTP does.

## Decision Outcome

Chosen option: the server and the browser, because one rule then holds for
every read, and the page keeps no copy of its own to go stale.

* Every GET of the page's API answers with an ETag and `private, no-cache`:
  the browser keeps the answer and asks the server before each use; the
  server answers 304 when it has not changed.
* An answer that holds a secret or a sign-in, such as a credential, a
  session or a subscription's sign-in code, is `no-store` and has no ETag.
* Signing out clears the browser's cache of the account.
* The page no longer tags or compares reads itself.

### Performance

* An unchanged read crosses the network as a 304 with no body.
* The page parses an unchanged answer again, from the browser's cache.

### Consequences

* Good, because no read can be reused unseen by the server.
* Bad, because the account's answers rest in the browser's disk cache until
  sign-out.
