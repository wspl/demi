---
status: proposed
---

# An integration sizes its Clouds and keeps some running

## Problem

A team's bot gives each person a Cloud, and one Cloud of its own that
watches the team's repositories around the clock. Every Cloud of the
server stops after an hour without a conversation's activity (the idle
window), so the watcher stops each night and its watch with it. Each Cloud
gets the same 2 CPUs and 2 GiB, which the manager's environment sets for
the whole server, and the disks of the account that made the integration,
which Settings › Users sets per account: the bot can give neither its
heavy build Clouds more nor its many idle ones less.

## Decision

### Only the master has integrations

Where 0149 let any user make an integration in Settings › Integrations, only
the master does: the section is in the master's rail alone, and each route of
`/integrations` answers anyone else `403 forbidden`. An integration sizes the
Clouds it runs on the server, and what a Cloud takes of the server is the
master's to give: with the master the only one who sizes, no size needs a
gate of its own.

A product no longer makes a Demi user of its own for its integration, as
0149 had it: every integration of a server infers with the master's
providers, in the master's shard, and keeps its attachments among the
master's. Demi keeps integrations of one server apart only where an agent
could reach what is not its own: each has its workspace, its capabilities,
its token and its Clouds. A product that needs its providers or its work
apart runs a server of its own, whose master it is.

### The integration's Clouds

Two settings of the integration reach every Cloud of it at its next start:

* `cloudIdleMinutes`, how long a Cloud runs without activity before it
  stops; none for the server's hour. The integration's Settings page and
  `PATCH /v1/integration` change it.
* `cloudSizing`, the CPU budget and memory limit the manager applies and the
  sizes of the system and home disks:

  ```rust
  pub struct CloudSizing {
      pub cpus: Option<u32>,
      pub memory_mib: Option<u32>,
      pub system_gb: Option<u16>,
      pub home_gb: Option<u16>,
  }
  ```

  A field that is none is the server's: the manager's `DEMI_MANAGED_CPUS`
  and `DEMI_MANAGED_MEM_MIB`, which the backend does not know, and a
  Cloud's default disks, 16 GB each: the disks an administrator sets per
  account in Settings › Users are that person's own Cloud's. The
  integration's Settings page changes it; the integration API only reads
  it.

A smaller disk size leaves a disk as it is, since the manager enlarges an
image and never shrinks one (`machine-manager/src/storage/mod.rs`).

* The backend names the CPU budget and the memory limit in each wake
  (`WakeParams.limits`), as it names the disks; the manager's settings
  give each that the wake leaves out, such as a person's own Cloud's. With
  the resource limits on, a wake that asks for more CPUs or memory than the
  host has is refused, and the work that woke the Cloud fails with why.
* A person's own Cloud keeps the server's settings and the account's disks.

### A Cloud that stays running

`CreateCloud` and `PATCH /v1/clouds/{cloud}` take `alwaysOn`. An always-on
Cloud starts once it turns always on and runs its init script, has no idle
stop, and after its lifetime cap's stop, which saves it, it starts again at
once. A backend that starts starts each always-on Cloud the manager does not
run, as after the server's own restart. A reset and a deletion stop it as
any other; one that turns off stops once it has been idle for the idle
window.

### What needs no setting

* A program's command (0225) and a conversation's work keep a Cloud awake
  while they run, so work in the background outlives no idle window.
* A Cloud's environment variables are lines in its `~/.profile`, which an
  agent's shell reads as a login shell does, written by the init script or
  a program's command.
* After a change of the init script, each Cloud takes the new script at
  its next start (0156); `POST /v1/clouds/{cloud}/reset` gives one a new
  system now.

## Spike

* The idle window is one value of the backend's tuning
  (`backend-user-shard/src/tuning.rs`, 60 minutes), read through
  `idle_window()` per Cloud; the lifetime cap is 24 hours
  (`backend-cloud/src/tuning.rs`).
* The CPU budget and the memory limit are manager settings for every
  sandbox (`machine-manager/src/config.rs`); the disks already travel in
  each wake (`Capacity`).
* An account's disk sizes are stored per user (`users.cloud_system_gb`,
  `cloud_home_gb`), set in Settings › Users.

## Prior art

* E2B takes a sandbox's timeout when it is created; Daytona takes its CPU,
  memory and auto-stop interval, which `0` turns off.
* GitHub Codespaces lets an organization set the idle timeout and the
  machine types its codespaces may use.

## Alternatives

* **Sizes per Cloud**: a program would size each Cloud it makes, though
  its Clouds are of one kind; `alwaysOn` alone is per Cloud, since one
  Cloud of many watches.
* **Sizes the integration sets, up to its account's**: a size could then
  only shrink a Cloud, never give a build Cloud more.
* **Server-wide settings in Settings**: the person's own Clouds and every
  integration's would still share one size.
* **Any user's integrations, sized by an administrator**: an administrator
  would need a page for every account's integrations, and a user's own
  integration would show sizes its owner cannot change.
* **Sizes on the account, beside its disks in Settings › Users**: every
  integration of an account would share one size.
* **A shard and attachments of its own per integration**: integrations of
  one server would be apart from each other and from the master's own work,
  at the cost of routing every integration route by the integration rather
  than its user; a product that needs that runs its own server.

## Open questions

* A program learns of no restart of its always-on Cloud, after which what
  it ran there is gone; an event of a Cloud's state on the socket would
  tell it.
