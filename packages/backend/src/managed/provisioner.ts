import type { ManagedHostOwner } from '../storage/control'

/** What a guest needs at boot, fresh at every provision and wake: where the backend is and the token that admits it. */
export interface BootArgs {
  backendUrl: string
  deviceToken: string
}

/**
 * The provisioner seam (`managed-hosts.md` § Provisioning): the VM and
 * nothing else. One owner has at most one guest; the provisioner keeps the
 * owner's home across hibernate, wake, destroy and backend restarts by
 * whatever means it has — the Firecracker implementation as an ext4 image
 * in the home-image store, the test fake as the directory it was given. The
 * lifecycle above never sees an image.
 *
 * Every path that ends a guest — hibernate, destroy, the backend closing,
 * a crash found at the next start — saves its home: the store holds the
 * current home whenever no guest is running.
 */
export interface ManagedHostProvisioner {
  /**
   * The backend starting: whatever a previous process left running or
   * unsaved is killed and saved before any guest is booted. Completes
   * before the lifecycle accepts its first need.
   */
  reconcile(): Promise<void>
  /** The first boot: `homeDir` is the owner's home, already materialised (the hostless tree, or empty). */
  provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void>
  /** A fresh guest over the home saved at the last hibernate or checkpoint. */
  wake(owner: ManagedHostOwner, boot: BootArgs): Promise<void>
  /**
   * Kills the guest and makes its home durable. `untouched` is the guest's
   * own report from the `sync` before the kill: nothing wrote to the home
   * since this boot, so the saved copy is current and the upload can be
   * skipped (`managed-hosts.md` § Home persistence).
   */
  hibernate(owner: ManagedHostOwner, report: { untouched: boolean }): Promise<void>
  /** The guest asked for a bigger home: the backing image becomes `bytes` large; the guest grows the filesystem afterwards. */
  growHome(owner: ManagedHostOwner, bytes: number): Promise<void>
  /** Saves the home of a running guest; the guest is paused for the copy. */
  checkpoint(owner: ManagedHostOwner): Promise<void>
  /** The guest is gone for good: killed if it runs, its home saved and kept (retention is a later item). */
  destroy(owner: ManagedHostOwner): Promise<void>
  /** The backend closing: every running guest killed and its home saved. */
  close(): Promise<void>
  /** Called once per guest death the provisioner did not cause itself. */
  onDeath(listener: (owner: ManagedHostOwner) => void): void
}

export function ownerKey(owner: ManagedHostOwner): string {
  return `${owner.kind}:${owner.id}`
}

/** The owner an `ownerKey` names; null for a string that is not one. */
export function ownerFromKey(key: string): ManagedHostOwner | null {
  const colon = key.indexOf(':')
  if (colon === -1) return null
  const kind = key.slice(0, colon)
  const id = key.slice(colon + 1)
  if ((kind !== 'conversation' && kind !== 'workspace') || id.length === 0) return null
  return { kind, id }
}
