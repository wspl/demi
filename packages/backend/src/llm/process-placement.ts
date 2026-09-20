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
  /** The machine user's home directory. */
  cwd: string
  /** The command context package operations on this machine run under. */
  context: CommandContext
  release(): void
}

type Candidate = (signal?: AbortSignal) => Promise<PlacedHost | null>

/**
 * The whole policy: for each kind of work, the machines to try in order. A
 * provider's process runs on the user's Cloud, whatever the conversation's
 * execution target is (`claude-cli.md` § Where it runs).
 */
const POLICY: Record<ProcessWork['kind'], ReadonlyArray<'cloud' | 'execution-target'>> = {
  inference: ['cloud'],
  account: ['cloud'],
}

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
   * The machines account work is placed on that can be asked something now.
   * It wakes nothing: a stopped Cloud is simply not among them.
   */
  async online(userId: string, scope: string): Promise<Array<PlacedHost & { name: string }>> {
    const [managed, { locale }] = await Promise.all([
      this.deps.control.getManagedDevice(userId),
      this.deps.control.getUserPreferences(userId),
    ])
    return (managed ? [managed] : []).flatMap((device) => {
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

  /**
   * Whether this kind of work is placed on the user's Cloud. A conversation
   * whose provider needs a process uses that Cloud as `provider` exactly when
   * its requests are placed there, so the lifecycle asks the same policy.
   */
  onCloud(kind: ProcessWork['kind']): boolean {
    return POLICY[kind].includes('cloud')
  }

  private candidates(work: ProcessWork): Candidate[] {
    return POLICY[work.kind].map((machine): Candidate => {
      if (machine === 'cloud')
        return async (signal) => work.kind === 'inference'
          ? this.cloud(
            await this.conversationUser(work.conversationId),
            `provider-process-${work.conversationId}`,
            await buildCommandContext(this.deps.control, work.conversationId, { kind: 'user' }),
            signal
          )
          : this.cloud(work.userId, `provider-${work.providerId}`, null, signal)
      return () => work.kind === 'inference'
        ? this.executionTarget(work.conversationId)
        : Promise.resolve(null)
    })
  }

  private async conversationUser(conversationId: string): Promise<string> {
    const conversation = await this.deps.control.getConversation(conversationId)
    if (!conversation)
      throw new Error(`No conversation ${conversationId}`)
    return conversation.userId
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
      cwd: host.identity.homeDir,
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
    context: CommandContext | null,
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
        context: context ?? {
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
