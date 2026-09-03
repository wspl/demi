import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { startTinyjsRunner, type TinyjsRunner } from '@demicodes/runner/testing'
import type { BootArgs, ManagedHostProvisioner } from '../../managed/provisioner'
import { ownerKey } from '../../managed/provisioner'
import type { ManagedHostOwner } from '../../storage/control'

interface Guest {
  owner: ManagedHostOwner
  homeDir: string
  stateDir: string
  runner: TinyjsRunner | null
  /** Set around a stop the provisioner itself performs, so the exit is not reported as a death. */
  stopping: boolean
}

/**
 * The provisioner seam over a local packed tinyjs runner: the "VM" is a
 * process with the owner's `homeDir` as its `HOME`, started as a managed
 * host with the pre-issued token. Hibernate stops the process and keeps the
 * directory; wake starts a new process over it; a process that exits on its
 * own is a guest death. `calls` is the record scenarios assert on.
 */
export class FakeProvisioner implements ManagedHostProvisioner {
  readonly guests = new Map<string, Guest>()
  readonly calls: string[] = []
  private readonly deathListeners: Array<(owner: ManagedHostOwner) => void> = []
  /** How long a checkpoint holds the guest "paused". */
  checkpointMs = 0

  async provision(owner: ManagedHostOwner, homeDir: string, boot: BootArgs): Promise<void> {
    this.calls.push(`provision:${ownerKey(owner)}`)
    const stateDir = await mkdtemp(join(tmpdir(), 'demi-fake-vm-state-'))
    const guest: Guest = { owner, homeDir, stateDir, runner: null, stopping: false }
    this.guests.set(ownerKey(owner), guest)
    await this.start(guest, boot)
  }

  async wake(owner: ManagedHostOwner, boot: BootArgs): Promise<void> {
    this.calls.push(`wake:${ownerKey(owner)}`)
    await this.start(this.guest(owner), boot)
  }

  async hibernate(owner: ManagedHostOwner): Promise<void> {
    this.calls.push(`hibernate:${ownerKey(owner)}`)
    await this.stop(this.guest(owner))
  }

  async checkpoint(owner: ManagedHostOwner): Promise<void> {
    this.calls.push(`checkpoint:${ownerKey(owner)}`)
    if (this.checkpointMs > 0) await new Promise((resolve) => setTimeout(resolve, this.checkpointMs))
  }

  async destroy(owner: ManagedHostOwner): Promise<void> {
    this.calls.push(`destroy:${ownerKey(owner)}`)
    await this.stop(this.guest(owner))
  }

  onDeath(listener: (owner: ManagedHostOwner) => void): void {
    this.deathListeners.push(listener)
  }

  /** The guest dies on its own: what a crashed VM looks like from above. */
  async kill(owner: ManagedHostOwner): Promise<void> {
    const guest = this.guest(owner)
    const runner = guest.runner
    if (!runner) return
    await runner.stop()
  }

  running(owner: ManagedHostOwner): boolean {
    return this.guests.get(ownerKey(owner))?.runner !== null
  }

  homeOf(owner: ManagedHostOwner): string {
    return this.guest(owner).homeDir
  }

  async close(): Promise<void> {
    for (const guest of this.guests.values()) await this.stop(guest)
  }

  private guest(owner: ManagedHostOwner): Guest {
    const guest = this.guests.get(ownerKey(owner))
    if (!guest) throw new Error(`no guest for ${ownerKey(owner)}`)
    return guest
  }

  private async start(guest: Guest, boot: BootArgs): Promise<void> {
    if (guest.runner) throw new Error(`guest ${ownerKey(guest.owner)} already runs`)
    const runner = await startTinyjsRunner({
      backendUrl: boot.backendUrl,
      stateDir: guest.stateDir,
      home: guest.homeDir,
      name: 'cloud',
      deviceToken: boot.deviceToken,
      managed: true,
    })
    guest.runner = runner
    guest.stopping = false
    void runner.exited.then(() => {
      if (guest.runner !== runner) return
      guest.runner = null
      if (!guest.stopping) for (const listener of this.deathListeners) listener(guest.owner)
    })
  }

  private async stop(guest: Guest): Promise<void> {
    const runner = guest.runner
    if (!runner) return
    guest.stopping = true
    await runner.stop()
    await runner.exited
    guest.runner = null
  }
}
