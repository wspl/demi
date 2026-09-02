import { join } from 'node:path'
import type { Block, UserContentBlock } from '@demicodes/core'
import { AgentClient, createWebSocketClientTransport, type ClientSessionEvent } from '@demicodes/agent'
import type { InferenceRequest } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { HOSTLESS_HOME } from '../../conversation/scoped-transport'
import type { TurnScript } from './model'
import type { World } from './world'

/** `hostless`, or `runner:<name>` for a device the world paired. */
export type Target = 'hostless' | `runner:${string}`

export type ShellOutputEvent = Extract<ClientSessionEvent, { type: 'shell_output' }>

export interface Turn {
  /** The tool results as the model received them, one text per tool call, in order. */
  received: string[]
  /** The shell_output frames of this turn. */
  shell: ShellOutputEvent[]
  /** The blocks this turn appended to the live transcript. */
  blocks: Block[]
  /** The inference requests this turn made, for assertions on what the model was shown. */
  requests: InferenceRequest[]
}

/**
 * One conversation of the world: created over the API, bound to its target,
 * its stream open through an AgentClient. `turn` scripts the model, sends,
 * and returns the turn's observation.
 */
export class Driver {
  readonly events: ClientSessionEvent[] = []
  private client!: AgentClient
  private socket!: WebSocket
  private seenToolResults = new Set<string>()

  private constructor(
    readonly world: World,
    readonly id: string,
    public target: Target,
  ) {}

  static async open(world: World, target: Target): Promise<Driver> {
    const { conversation } = await world.api<{ conversation: { id: string } }>('/api/conversations', {})
    const driver = new Driver(world, conversation.id, target)
    if (target !== 'hostless') await driver.switchTo(target)
    await driver.attach()
    return driver
  }

  get label(): string {
    return `${this.id} on ${this.target}`
  }

  /** Opens a client on the conversation's stream; the previous client, if any, is dropped without a close frame. */
  async attach(): Promise<void> {
    const socket = new WebSocket(`${this.world.url.replace('http', 'ws')}/api/conversations/${this.id}/stream`)
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true })
      socket.addEventListener('error', () => reject(new Error('stream connect failed')), { once: true })
    })
    this.socket = socket
    this.client = new AgentClient(createWebSocketClientTransport(socket as never))
    this.client.subscribe((event) => void this.events.push(event))
    await this.client.open(this.world.selection, '/ignored-by-server', 'ignored')
  }

  /** Drops the raw socket, as a browser refresh does. */
  async detach(): Promise<void> {
    if (this.socket.readyState === WebSocket.OPEN) this.socket.close()
  }

  /** Rebinds the conversation's target over PATCH (a turn-boundary switch). */
  async switchTo(target: Target): Promise<void> {
    const workspaceId = target === 'hostless' ? null : this.world.device(target.slice('runner:'.length)).workspaceId
    await this.world.api(`/api/conversations/${this.id}`, { workspaceId }, 'PATCH')
    this.target = target
  }

  script(...turns: TurnScript[]): void {
    this.world.model.script(this.id, ...turns)
  }

  /** Scripts the model for this turn, sends the user text, waits for the turn to end. */
  async turn(script: { model: TurnScript[]; text?: string; content?: UserContentBlock[] }): Promise<Turn> {
    this.script(...script.model)
    const begin = this.begin()
    await this.client.send(script.content ?? [{ type: 'text', text: script.text ?? 'go' }])
    return this.observe(begin)
  }

  /** Starts a turn without waiting for it: the detach and restart scenarios. */
  startTurn(script: { model: TurnScript[]; text?: string }): { begin: TurnBegin; done: Promise<void> } {
    this.script(...script.model)
    const begin = this.begin()
    const done = this.client.send([{ type: 'text', text: script.text ?? 'go' }]).catch(() => {})
    return { begin, done }
  }

  begin(): TurnBegin {
    return { events: this.events.length, blocks: this.transcript().length, requests: this.world.model.requests.length }
  }

  /** The observation of a turn that began at `begin`; `received` covers tool results not observed before. */
  observe(begin: TurnBegin): Turn {
    const requests = this.world.model.requests.slice(begin.requests).filter((request) => request.sessionId === this.id)
    const received: string[] = []
    for (const request of requests) {
      for (const item of request.items) {
        if (item.type !== 'tool_result' || this.seenToolResults.has(item.toolUseId)) continue
        this.seenToolResults.add(item.toolUseId)
        received.push(item.output.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join('\n'))
      }
    }
    return {
      received,
      shell: this.events.slice(begin.events).filter((event): event is ShellOutputEvent => event.type === 'shell_output'),
      blocks: this.transcript().slice(begin.blocks),
      requests,
    }
  }

  transcript(): Block[] {
    return this.client.transcript().blocks
  }

  /** The last assistant text of the live transcript. */
  lastText(): string {
    const block = this.transcript().filter((b) => b.type === 'text').at(-1)
    return block?.type === 'text' ? block.text : ''
  }

  /** Where the target keeps a file under the conversation's home. */
  filePath(relative: string): string {
    if (this.target === 'hostless') return join(this.world.dataDir, 'virtual', this.id, HOSTLESS_HOME, relative)
    return join(this.world.device(this.target.slice('runner:'.length)).home, relative)
  }

  get agent(): AgentClient {
    return this.client
  }
}

export interface TurnBegin {
  events: number
  blocks: number
  requests: number
}

/** The allowed differences between the two targets, keyed by what a scenario asks about. */
export function expected(target: Target) {
  const hostless = target === 'hostless'
  return {
    hostCurrent: hostless ? 'host: virtual' : `on device "${target.slice('runner:'.length)}"`,
    outputPath: !hostless,
  }
}

/**
 * Turn scripts shaped as a real provider answers: every request ends with a
 * `response` carrying usage, so each one lands in the ledger.
 */
export const model = {
  /** A request answered with one shell_exec call. */
  shell: (toolUseId: string, script: string, timeoutMs = 10_000): TurnScript => [
    events.toolCall(toolUseId, 'shell_exec', { script, timeoutMs }),
    events.response(),
  ],
  /** A request answered with one tool call of any kind. */
  tool: (toolUseId: string, toolName: string, input: unknown): TurnScript => [events.toolCall(toolUseId, toolName, input), events.response()],
  /** A request answered with text; the turn ends. */
  say: (text: string): TurnScript => [events.text(text), events.response()],
}
