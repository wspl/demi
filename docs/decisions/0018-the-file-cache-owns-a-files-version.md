---
status: accepted
---

# The file cache owns a file's version

## Context and Problem Statement

An image the agent delivered opens in the work panel's media viewer as its
file's latest version, or as the attachment's own copy when the file is
gone. Opening it asked the backend whether the file is still there; the
answer named the file's version and whether it still holds the copy's
bytes, so an unchanged picture shows from the copy the page already holds.

The viewer kept that answer as its own description of the file, beside the
file cache's description and the Host's change reports. Two owners of one
fact disagreed whenever their readings arrived out of order: a picture
opened again showed the version of its first opening, a newer answer was
ignored by a cache that counted itself current, an answer the Host had
already reported changed showed the old copy first, and the page asked the
Host for a version it no longer had.

## Considered Options

* Keep the viewer's own description and order it against the cache's.
* Make the file cache the only owner of a file's version, and the answer to
  an opening one more reading of it.

## Decision Outcome

Chosen option: the file cache alone, because a fact with one owner cannot
disagree with itself.

* The answer to an opening enters the file cache as a reading of the file,
  ordered with the cache's own readings by the Host's number. A reading
  older than one the cache holds, or than a change the Host reported, is
  dropped.
* The viewer shows what the cache says and holds no description of its own.
* A file the Host reported changed, whose new reading is on its way, opens
  only once that reading arrives, never as its old version or the copy.
* The copy shows only while the cache's version is the one the answer said
  holds the copy's bytes.
* Each opening waits for its own picture; a newer version of a picture on
  screen replaces it in place.

### Consequences

* Good, because no order of answers, readings and change reports shows an
  older version than the newest the page has heard of.
* Bad, because an opening right after a change waits for the new reading
  instead of showing something at once.
