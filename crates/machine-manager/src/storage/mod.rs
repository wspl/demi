//! A device's storage: its committed generations, its working pair and the
//! bases they stand on.
//!
//! A generation names two immutable ext4 image files, system and home, with the
//! capacities the filesystems have. The capacities are read from the image's
//! superblock (a documented on-disk format) rather than from a tool's output,
//! and they are not used bytes, host allocation or quota. Only private working
//! copies are ever mounted writable. A record that fails to decode is an error;
//! the manager never repairs it.
//!
//! # The layers
//!
//! - The base, a pinned read-only directory, holds the OS, the shipped tools,
//!   the runner and init. Stop and wake keep the exact base, a reset selects
//!   the reset base, and an ordinary wake never silently upgrades one.
//! - The system image holds OverlayFS's upper and work directories (everything
//!   outside the separate mounts), which a reset replaces with an empty layer.
//! - The home image is `/home`, which a reset keeps.
//! - The runtime mounts (`/run`, `/tmp`, `/dev/shm`, the boot credential, the
//!   resolver configuration) are made anew at every boot.
//!
//! A base fixes its CPU architecture: moving persisted state between
//! architectures is outside the contract.
//!
//! # The runner's working files
//!
//! The runner's working files are on the system image, in directories the image
//! makes for the `demi` user:
//!
//! - The job root `/var/lib/demi/jobs`.
//! - The temporary directory `/var/lib/demi/tmp`.
//! - The Host log `/var/log/demi`.
//!
//! None belongs in `/run` or `/tmp`, 256 MiB of memory each: a few busy jobs'
//! kept output, or one build's temporary files, can fill them, and the Host log
//! must outlive the stop that empties `/run`. None belongs under `/home`
//! either, which is the user's volume and survives a reset, where these are
//! Demi's working files.
//!
//! The image makes only `/var/lib/demi`: a Cloud pinned to an older base keeps
//! that base's `/var` while `/opt/demi` brings the new runner, so a runner that
//! needed `tmp` from the image could not start on a Cloud pinned to 0.1.11's
//! base.
//!
//! The runner's artifact cache is in the home, so what the image does not hold
//! is downloaded once and survives stops, wakes and resets.
//!
//! # Capacities
//!
//! - The capacities are fixed: the backend names them at each wake from its
//!   account's setting and they never grow while a Cloud runs.
//! - A smaller image is enlarged at the next wake, before it is mounted, and a
//!   larger one keeps its size, since shrinking a filesystem risks its data.
//! - A full filesystem gets `ENOSPC` like any full disk and its working data
//!   stays as it was.
//! - The host's own free space is promised by no capacity.
//!
//! # Checkpoints
//!
//! A checkpoint is a disk operation, not a gVisor memory checkpoint. Pausing
//! the sandbox alone is no persistence barrier: shared host-backed writes, the
//! filesystems' flush and freeze and durable publication are all needed, and an
//! unresponsive runtime cannot produce a successful live checkpoint. The
//! failure is reported, never covered by silently restarting the user's
//! conversation browser.
//!
//! Hibernate stops every writer first and must work without a healthy runner or
//! user shell. Neither path promises more than filesystem crash consistency:
//! unsynced application buffers can be lost, and nothing is atomic across files
//! or volumes.
//!
//! # Recovery
//!
//! Recovery rests on one rule, that volumes of different generations are never
//! combined.
//!
//! - Before a publication the prior committed pair and the newer working pair
//!   are both kept. After it the new committed pair is used.
//! - Working state that holds newer writes is recovered before a wake and never
//!   overwritten from an older generation.
//! - A save that fails, or a full storage, keeps the working data and the
//!   error, releases execution resources where that is safe and blocks a stale
//!   boot.
//! - Failed working data is never garbage, and staging data goes only once
//!   reconciliation establishes that no pending operation needs it.

pub mod archive;
#[cfg(target_os = "linux")]
pub mod base;
#[cfg(target_os = "linux")]
pub mod clone;
pub mod durable;
pub mod ext4;
#[cfg(target_os = "linux")]
pub mod skeleton;
pub mod state;
pub mod store;
pub mod working;
