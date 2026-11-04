---
status: proposed
---

# A shared folder has its own size, and a Cloud may have no CPU or memory limit

## Problem

A product keeps its users' repositories in its integration's shared
folder: 400 GB of them. The shared folder is made at the size of a Cloud's
home disk (0156), so the administrator sets every Cloud's home disk to
450 GB, though each person's home holds a few GB; the setting says the
wrong thing, and a Cloud's home could grow to a size nobody meant.

The same product runs heavy builds on some Clouds and wants them to take
what the host has. An integration's sizing (0227) names a CPU budget and a
memory limit, or leaves each to the server's, 2 CPUs and 2048 MiB: no value
says "no limit", so the administrator writes the host's numbers and
changes them when the host changes.

## Decision

### The shared folder's size

`cloudSizing` takes `sharedGb`, the size of the integration's shared
folder; none for 16 GB, a Cloud disk's default. It is a size of the
server's disk, set where the others are, in the integration's Settings page
by the master, and the integration API reads it (0227).

* The folder is made at its size at its first mount, as now.
* A larger size enlarges it at its next mount, the first after no Cloud of
  the integration ran; a smaller one leaves it as it is, as a disk is
  (`machine-manager/src/storage/shared.rs`). So every quota takes effect
  at a start, as 0227 has it.
* An integration whose Clouds never all stop at once, such as one with an
  always-on Cloud, keeps its folder's size until they do. That is the
  cost of one rule for every quota.
* A folder made before this keeps its size, the home disk's then: none
  shrinks.

The folder's image is sparse, so a large size takes the host's disk only
as files fill it.

### No limit

`cpus` and `memoryMib` take `"unlimited"` beside a number:

```rust
/// A CPU budget or a memory limit an integration names.
#[serde(untagged)]
pub enum CloudLimit {
    /// This many CPUs, or MiB.
    At(NonZeroU32),
    /// None: the Cloud takes what the host has.
    Unlimited(Unlimited), // "unlimited"
}

pub struct CloudSizing {
    pub cpus: Option<CloudLimit>,
    pub memory_mib: Option<CloudLimit>,
    pub system_gb: Option<u16>,
    pub home_gb: Option<u16>,
    pub shared_gb: Option<u16>,
}
```

* None is still the server's setting, as 0227 has it; `"unlimited"` is no
  limit; a number is that limit. Any other value is refused with what a
  right one is.
* With the manager's resource limits on, an unlimited Cloud's cgroup has
  no CPU quota or memory limit, so it sees the host's memory as its total;
  its process limit stays. With them off every Cloud already has none.
* The wake names it (`WakeParams.limits`), whose `SandboxLimits` takes the
  same three values, and the manager's wire version goes to 9.
* The disks take no `"unlimited"`: a disk is an image of a fixed size, and
  a large sparse size serves where one is wanted.
* A person's own Cloud keeps the server's limits and its account's disks,
  as 0227 has it.

### What the server gives

The page shows what a Cloud gets, not only what the integration names, so
the backend has to know the server's sizes:

* The manager tells the backend, when a connection opens, the host's CPUs
  and memory and the limits a Cloud gets when its wake names none, or that
  it limits no Cloud (`ManagerEvent::ServerResources`). The backend keeps
  them and never asks, as it keeps a Cloud's status.
* `GET /integrations/server-sizes` answers the master's page with each
  size an integration's Cloud gets when the integration leaves it to the
  server: the CPUs and memory the manager said, 16 GB for each disk and
  the shared folder, the server's idle window; and the host's CPUs and
  memory, which a Cloud without a limit may use.

### The Settings page

The integration's Clouds group shows each limit as the size fields show
theirs, with CPUs and Memory a pop-up button first, as macOS's Time Machine
shows its disk usage limit, and the Shared Folder group gains the folder's
size under its path. Every field shows the amount a Cloud gets:

```text
Shared Folder                     [ ~/shared           ]
Shared Folder Size                [ 500      ] GB

Sleep After                       [ 60       ] minutes
CPUs                [Default   ▾] [ 2        ]
Memory              [Custom    ▾] [ 8192     ] MiB
System Disk                       [ 16       ] GB
Home Disk                         [ 16       ] GB
```

* The pop-up offers Default, No Limit and Custom. Its field always shows:
  dimmed, with the server's limit under Default and the host's whole under
  No Limit, as macOS's Network settings dim the address DHCP gave; under
  Custom, the user edits it.
* Choosing Default or No Limit saves at once. Choosing Custom saves the
  amount the field showed as the integration's own, which the user then
  changes; before the server has said its sizes, Custom saves once the
  field holds a number. A Custom field left empty goes back to Default,
  as an empty size field is the server's.
* An empty size field shows the server's size as its placeholder.
* The Clouds group's description says, as it does, that each Cloud takes
  these at its next start; Shared Folder Size's says that the folder
  takes a larger size once no Cloud of the integration runs.

## Spike

None: a cgroup without a CPU quota or a memory limit is what the manager
runs with its limits off, and the folder's size is the capacity its image
is made and enlarged at now.

## Prior art

* Docker and Kubernetes give a container no CPU or memory limit when none
  is named
  ([Docker](https://docs.docker.com/engine/containers/resource_constraints/),
  [Kubernetes](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/));
  here none already means the server's setting, so no limit is a value of
  its own.
* Proxmox VE's `cpulimit` of 0 is no limit
  ([Proxmox VE](https://pve.proxmox.com/wiki/Manual:_qm.conf)); a word
  reads plainer than a number that means its opposite.
* macOS's Time Machine options choose a disk usage limit from a pop-up
  button, None or Custom, and Custom shows the size.
* macOS's Network settings dim the fields a choice fills, such as the
  address DHCP gave, and let the user edit them only under Manually.

## Alternatives

* The shared folder at a home disk's size, as now: one setting for two
  disks that serve different things.
* A disk of no limit: an image as large as the host's disk would need its
  metadata made for terabytes at once, for what a large size gives.
* A shared folder that grows at once, while its Clouds run: a quota that
  takes effect at once where every other waits for a start.

## Open questions

None.
