import { mkdtemp, rm, mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { startTxikiRunner, type TxikiRunner } from '@demicodes/runner/testing'
import type {
  BootArgs,
  ManagedHostProvisioner
} from '../../managed/provisioner'

interface Guest {
  owner: string
  homeDir: string
  stateDir: string
  runner: TxikiRunner | null
  /**
   * Set around a stop the provisioner itself performs, so the exit is not
   * reported as a death.
   */
  stopping: boolean
}

/**
 * The provisioner seam over a local packed txiki.js runner: the "VM" is a
 * process with the owner's `homeDir` as its `HOME`, started as a managed
 * host with the pre-issued token. Hibernate stops the process and keeps the
 * directory; wake starts a new process over it; a process that exits on its
 * own is a guest death. `calls` is the record scenarios assert on.
 */
export class FakeProvisioner implements ManagedHostProvisioner {
  readonly guests = new Map<string, Guest>()
  readonly calls: string[] = []
  private readonly deathListeners: Array<(owner: string) => void> = []
  /** How long a checkpoint holds the guest "paused". */
  checkpointMs = 0

  /** Nothing to settle: the fake's guests never outlive the test process. */
  async reconcile(): Promise<void> {}

  async currentBaseVersion(): Promise<string> {
    return 'test-base'
  }
  async imageState(id: string) {
    return this.guests.has(id) ? {
      generation: 'test',
      baseVersion: 'test-base',
      resetId: null,
      systemBytes: 1024 ** 3,
      homeBytes: 1024 ** 3
    } : null
  }
  async wake(owner: string, boot: BootArgs): Promise<void> {
    this.calls.push(`wake:${owner}`)
    if (!this.guests.has(owner)) {
      const homeDir = await mkdtemp(join(tmpdir(), 'demi-fake-home-'))
      const stateDir = await mkdtemp(join(tmpdir(), 'demi-fake-state-'))
      this.guests.set(owner, {
        owner,
        homeDir,
        stateDir,
        runner: null,
        stopping: false
      })
    }
    await this.start(this.guest(owner), boot)
  }
  async hibernate(owner: string): Promise<void> {
    this.calls.push(`hibernate:${owner}`)
    const guest = this.guests.get(owner)
    if (guest)
      await this.stop(guest)
  }
  async growVolume(
    owner: string,
    volume: 'system' | 'home',
    bytes: number
  ): Promise<void> {
    this.calls.push(`grow:${owner}:${volume}:${bytes}`)
  }
  async reset(
    owner: string,
    operationId: string,
    baseVersion: string
  ): Promise<void> {
    this.calls.push(`reset:${owner}:${operationId}:${baseVersion}`)
    const guest = this.guests.get(owner)
    if (guest) {
      await this.stop(guest)
      await rm(guest.stateDir, { recursive: true, force: true })
      await mkdir(guest.stateDir)
    }
  }

  async checkpoint(owner: string): Promise<void> {
    this.calls.push(`checkpoint:${owner}`)
    if (this.checkpointMs > 0)
      await new Promise((resolve) => setTimeout(resolve, this.checkpointMs))
  }

  onDeath(listener: (owner: string) => void): void {
    this.deathListeners.push(listener)
  }

  /** The guest dies on its own: what a crashed VM looks like from above. */
  async kill(owner: string): Promise<void> {
    const guest = this.guest(owner)
    const runner = guest.runner
    if (!runner)
      return
    await runner.stop()
  }

  running(owner: string): boolean {
    return this.guests.get(owner)?.runner !== null
  }

  homeOf(owner: string): string {
    return this.guest(owner).homeDir
  }

  async close(): Promise<void> {
    for (const guest of this.guests.values())
      await this.stop(guest)
  }

  private guest(owner: string): Guest {
    const guest = this.guests.get(owner)
    if (!guest)
      throw new Error(`no guest for ${owner}`)
    return guest
  }

  private async start(guest: Guest, boot: BootArgs): Promise<void> {
    if (guest.runner)
      throw new Error(`guest ${guest.owner} already runs`)
    const runner = await startTxikiRunner({
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
      if (guest.runner !== runner)
        return
      guest.runner = null
      if (!guest.stopping)
        for (const listener of this.deathListeners)
          listener(guest.owner)
    })
  }

  private async stop(guest: Guest): Promise<void> {
    const runner = guest.runner
    if (!runner)
      return
    guest.stopping = true
    await runner.stop()
    await runner.exited
    guest.runner = null
  }
}
