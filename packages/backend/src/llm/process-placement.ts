import type { CommandContext } from '@demicodes/command-protocol'
import type { RemoteHost } from '@demicodes/host-remote'
import type { ConversationTargets } from '../conversation/target'
import type { ManagedHosts } from '../managed/lifecycle'
import { buildCommandContext, DEFAULT_LOCALE } from '../runner/command-context'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService } from '../storage/control'
import { administrativeStore } from '../storage/host-store'

/** Work that needs a provider's process on some machine (`claude-cli.md` § Where it runs). */
export type ProcessWork =
  /** A conversation's request. */
  | { kind: 'inference'; conversationId: string }
  /** Work that belongs to no conversation: installing, testing a connection. */
  | { kind: 'account'; userId: string; providerId: string }

/** The machine chosen for a piece of work, held until `release`. */
export interface PlacedHost {
  host: RemoteHost
  deviceId: string
  cwd: string
  /** The command context package operations on this machine run under. */
  context: CommandContext
  release(): void
}

type Candidate = (signal?: AbortSignal) => Promise<PlacedHost | null>

/**
 * Decides the machine for a provider's process. Callers say what the work is
 * and take the Host they are given; the order of candidates below is the whole
 * policy, and changing where something runs changes nothing else.
 */
export class ProcessPlacement {
  constructor(private readonly deps: {
    control: ControlService
    targets: Pick<ConversationTargets, 'hostFor'>
    managedHosts: ManagedHosts
    registry: RunnerRegistry
  }) {}

  /** The first candidate that is available; the work's own error when none is. */
  async place(work: ProcessWork, signal?: AbortSignal): Promise<PlacedHost> {
    for (const candidate of this.candidates(work)) {
      const placed = await candidate(signal)
      if (placed)
        return placed
    }
    throw new Error('No machine is available for this work')
  }

  /**
   * The user's machines that can be asked something now. It wakes nothing: a
   * stopped Cloud and an offline device are simply not among them.
   */
  async online(userId: string, scope: string): Promise<Array<PlacedHost & { name: string }>> {
    const [paired, managed, { locale }] = await Promise.all([
      this.deps.control.listDevices(userId),
      this.deps.control.getManagedDevice(userId),
      this.deps.control.getUserPreferences(userId),
    ])
    return [...paired, ...(managed ? [managed] : [])].flatMap((device) => {
      const home = this.deps.registry.deviceIdentity(device.id)?.homeDir
      if (!home || !this.deps.registry.deviceOnline(device.id))
        return []
      return [{
        name: device.name,
        host: this.deps.registry.hostFor(
          { deviceId: device.id, path: home },
          scope,
          administrativeStore
        ),
        deviceId: device.id,
        cwd: home,
        context: {
          conversation: scope,
          caller: { kind: 'user' as const },
          locale: locale ?? DEFAULT_LOCALE
        },
        release: () => {},
      }]
    })
  }

  private candidates(work: ProcessWork): Candidate[] {
    return work.kind === 'inference'
      ? [() => this.executionTarget(work.conversationId)]
      : [signal => this.cloud(work.userId, `provider-${work.providerId}`, signal)]
  }

  /** The conversation's execution target, woken or refused as any of its work is. */
  private async executionTarget(conversationId: string): Promise<PlacedHost> {
    const host = await this.deps.targets.hostFor(conversationId)
    const deviceId = this.deps.registry.deviceOf(host)
    if (!deviceId)
      throw new Error('The execution target has no device')
    return {
      host,
      deviceId,
      cwd: host.defaultCwd,
      context: await buildCommandContext(
        this.deps.control,
        conversationId,
        { kind: 'user' }
      ),
      release: () => {},
    }
  }

  /** The user's Cloud, which every deployment has and Demi may wake at will. */
  private async cloud(
    userId: string,
    scope: string,
    signal?: AbortSignal
  ): Promise<PlacedHost> {
    const device = await this.deps.control.getOrCreateCloudDevice(userId)
    const release = await this.deps.managedHosts.enter(device, signal)
    try {
      const home = this.deps.registry.deviceIdentity(device.id)?.homeDir
      if (!home)
        throw new Error('Cloud did not report its home directory')
      const { locale } = await this.deps.control.getUserPreferences(userId)
      return {
        host: this.deps.registry.hostFor(
          { deviceId: device.id, path: home },
          scope,
          administrativeStore
        ),
        deviceId: device.id,
        cwd: home,
        context: {
          conversation: scope,
          caller: { kind: 'user' },
          locale: locale ?? DEFAULT_LOCALE
        },
        release,
      }
    } catch (error) {
      release()
      throw error
    }
  }
}
