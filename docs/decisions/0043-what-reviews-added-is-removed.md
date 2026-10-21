---
status: accepted
---

# What reviews added beyond their designs is removed

## Context and Problem Statement

Reviews of several work packages reported situations that need a crafted
interleaving, a crash at one moment, two versions of a service at once, or
names that differ only by case or scheme. Each was fixed with a mechanism
and its tests, and some added a rule to the package's decision record.
No user meets these situations in normal use, and the designs the user
agreed did not ask for them.

## Considered Options

* Keep each mechanism and its tests.
* Remove each mechanism, and keep what a user meets in normal use.

## Decision Outcome

The mechanisms are removed, and the behavior returns to what each design
asks:

* 0033: background loading starts after the first conversation is drawn,
  whatever a shown region waits for. The sentence "It never competes with
  what a shown region is waiting for." no longer holds.
* 0034: an installation's directory is its readable name and the hash, not
  cut. The runner does not check which backend an installation belongs to
  before it acts on it; the installers keep their check.
  The sentence about the cut, and the one that nothing acts on another
  backend's installation, no longer hold.
* 0035: a resume whose child last ran on a provider entry that no longer
  exists is refused, as it was before 0035. The rule that the old
  provider is not a condition of going on no longer holds.
* 0011: a disconnected device's wait starts once its cleanup ends, not the
  moment its connection fails.
* 0016: browser actions keep their own short table of roles, not a copy of
  Playwright's; the click guard is not released while a dialog is open;
  and a frame's session whose cleanup fails is not kept for a later
  release.
* 0030: when two users' pulls of a shared skill interleave, its shown
  commit may step back until the next check.
* An automation's URL refuses an unknown address and a wrong key with the
  same answer, without spending equal time on both.
* A project's workflow files are read through links like any file, and a
  path in a workflow is either absolute or joined to its directory, on
  Windows too.

### Performance

* No change in normal use; less code runs on every Host read of a
  workflow.

### Consequences

* Good, because each package does what its design asks and no more.
* Bad, because a workflow, or a `.demi` directory, that is a link is not
  watched for changes to its target.
