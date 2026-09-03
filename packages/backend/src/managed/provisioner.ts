import type { ManagedHostOwner } from '../storage/control'

/** What a guest needs at boot, fresh at every provision and wake: where the backend is and the token that admits it. */
export interface BootArgs {
  backendUrl: string
  deviceToken: string
}

/**
 * The provisioner seam (`managed-hosts.md` § Provisioning): the VM and
 * nothing else. One owner has at most one guest; the provisioner keeps the
 * owner's home across hibernate and wake by whatever means it has — the
 * Firecracker implementation as an ext4 image in the home-image store, the
 * test fake as the directory it was given. The lifecycle above never sees
 * an image.
 */
export interface ManagedHostProvisioner {
  /** The first boot: `homeDir` is the owner's home, already materialised (the hostless tree, or empty). */
  provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void>
  /** A fresh guest over the home saved at the last hibernate or checkpoint. */
  wake(owner: ManagedHostOwner, boot: BootArgs): Promise<void>
  /** Kills the guest and makes its home durable. */
  hibernate(owner: ManagedHostOwner): Promise<void>
  /** Saves the home of a running guest; the guest is paused for the copy. */
  checkpoint(owner: ManagedHostOwner): Promise<void>
  /** The guest is gone for good; whether the home is kept is retention policy, not the caller's concern. */
  destroy(owner: ManagedHostOwner): Promise<void>
  /** Called once per guest death the provisioner did not cause itself. */
  onDeath(listener: (owner: ManagedHostOwner) => void): void
}

export function ownerKey(owner: ManagedHostOwner): string {
  return `${owner.kind}:${owner.id}`
}
