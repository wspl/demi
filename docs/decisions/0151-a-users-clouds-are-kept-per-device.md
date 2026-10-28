---
status: proposed
---

# A user's Clouds are kept per device

## Problem

An integration (0149) gives each person it serves a Cloud of their own: a
group chat's bot gives Alex one and Bo one, so each works with their own
files and sign-ins. A Demi user, however, holds exactly one Cloud:

* the user's shard keeps one Cloud machine
  (`crates/backend-cloud/src/machine.rs`), beside the master's Claude Code
  machine;
* the control database allows one managed device per user and kind
  (`devices_one_machine`);
* a conversation targets "the user's Cloud" by kind, never a Cloud by its
  device.

## Decision

A user holds any number of Clouds. Each is a managed device with its own
machine: its phase, gate, capacity permit, idle watch, reset, crash-loop
record and lifetime cap, as the one Cloud has them today.

* The user's own Cloud stays what it is: the one the web app shows, starts
  and resets, the target a new conversation takes by default, and the one
  `/cloud` names. Nothing on any page changes.
* Other Clouds are made only by an integration (0149) and named by their
  device. A conversation targets any of them by its device.
* Every running Cloud holds a permit of the machine's shared capacity, as
  today; a user's several Clouds take several permits.
* The Claude Code machine stays one, the master's.

**The design.**

* `Cloud` in the shard keeps its machines by device instead of one slot,
  and the Claude Code machine beside them. Every operation of `CloudShard`
  and its access, reset, status and maintenance takes the device it acts
  on.
* The control database stops counting Clouds per user: the user's record
  names their own Cloud's device, and `devices_one_machine` holds for the
  Claude Code machine alone. A migration names each user's existing Cloud.
* A conversation's target keeps its `cloud` kind for the user's own Cloud,
  and targets another Cloud as it targets a paired device, by its id.
* The runner's boot credential, the machine manager's socket and the image
  are per device already and do not change.

## Spike

None.

## Prior art

* [GitHub Codespaces](https://docs.github.com/en/codespaces/about-codespaces/what-are-codespaces):
  a user holds many codespaces, each with its own lifecycle, idle
  [timeout](https://docs.github.com/en/codespaces/setting-your-user-preferences/setting-your-timeout-period-for-github-codespaces)
  and storage, and one machine type quota across them.

## Alternatives

* **A Demi user per Cloud:** an identity per person an integration serves,
  which 0149 refuses.
* **Clouds outside the user's shard:** their state would have a second
  owner beside the shard that runs their conversations.

## Open questions

None.
