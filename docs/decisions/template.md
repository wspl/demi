---
status: proposed
---

# A sentence that says what the decision makes true

Supersedes, in part: NNNN (its title), where it says what no longer holds.

## Summary

One paragraph: what changes, for whom, and why.

## Motivation

What happens today, in a concrete case the reader can picture, with the
measurements that show it, and what it costs the user.

## Guide-level explanation

What a user sees and does, or what a model reads and runs, after the
change, told as the user or the model would meet it. A change on screen
draws the screen as an ASCII sketch.

## Reference-level explanation

The design in enough detail to build it from:

* Interfaces: each route with its method, path, parameters, body, answer
  and status codes; each socket frame and message; each command and its
  output; written as the Rust or TypeScript types they become.
* State: what is stored, where, under which version, and who owns it.
* Behavior: order, concurrency, errors, limits and edge cases, each as a
  rule the code keeps.

## Upgrade and version skew

What happens when parts of different versions meet: an open page of the
old build and the new backend, an old runner and a new backend, data
written by the release before. What migrates, and what is read as it is.

## Performance

What the change costs and saves, measured or to be measured, before and
after.

## Drawbacks

Why this might not be done.

## Rationale and alternatives

The options considered, and why this one; what not doing it means.

## Prior art

How leading projects of the kind do it, with sources, and where this design
departs from them.

## Unresolved questions

What is still open, for the user to decide before or while it is built.
