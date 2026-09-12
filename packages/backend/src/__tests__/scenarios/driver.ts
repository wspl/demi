import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { Block, UserContentBlock } from '@demicodes/core'
import {
  AgentClient,
  createWebSocketClientTransport,
  type ClientSessionEvent
} from '@demicodes/agent'
import type { InferenceRequest, ProviderEvent } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { delay, waitFor } from '@demicodes/utils'
import type { z } from 'zod'
import { type uploadRefBlockSchema } from '@demicodes/product-contracts'
import { ATTACHMENTS_DIR } from '../../conversation/attachment-refs'
import { CLOUD_HOME } from '../../conversation/execution-target'
import type { TurnScript } from './model'
export type { TurnScript } from './model'
import type { World } from './world'

/** `cloud`, or `runner:<name>` for a device the world paired. */
export type Target = 'cloud' | `runner:${string}`

export type ShellOutputEvent = Extract<ClientSessionEvent, { type: 'shell_output' }>

export interface Turn {
  /**
   * The tool results as the model received them, one text per tool call, in
   * order.
   */
  received: string[]
  /** The shell_output frames of this turn. */
  shell: ShellOutputEvent[]
  /** The blocks this turn appended to the live transcript. */
  blocks: Block[]
  /**
   * The inference requests this turn made, for assertions on what the model was
   * shown.
   */
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
    const { conversation } = await world.api<{ conversation: { id: string } }>(
      '/api/conversations',
      { id: crypto.randomUUID() }
    )
    const driver = new Driver(world, conversation.id, target)
    if (target !== 'cloud')
      await driver.switchTo(target)
    await driver.attach()
    return driver
  }

  static async attachExisting(world: World, id: string, target: Target): Promise<Driver> {
    const driver = new Driver(world, id, target)
    await driver.attach()
    world.drivers.push(driver)
    await waitFor(() => driver.agent.transcriptVersion() !== null)
    return driver
  }

  get label(): string {
    return `${this.id} on ${this.target}`
  }

  /**
   * Opens a client on the conversation's stream; the previous client, if any,
   * is dropped without a close frame.
   */
  async attach(): Promise<void> {
    const socket = this.world.backend.session.socket(
      `/api/conversations/${this.id}/stream`
    )
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true })
      socket.addEventListener(
        'error',
        () => reject(new Error('stream connect failed')),
        { once: true }
      )
    })
    this.socket = socket
    this.client = new AgentClient(
      createWebSocketClientTransport(socket as never)
    )
    this.client.subscribe((event) => void this.events.push(event))
    await this.client.open(
      this.world.selection,
      '/ignored-by-server',
      'ignored'
    )
  }

  /** Drops the raw socket, as a browser refresh does. */
  async detach(): Promise<void> {
    if (this.socket.readyState === WebSocket.OPEN)
      this.socket.close()
  }

  /** Rebinds the conversation's target over PATCH (a turn-boundary switch). */
  async switchTo(target: Target): Promise<void> {
    const workspaceId = target === 'cloud'
      ? null
      : this.world.device(target.slice('runner:'.length)).workspaceId
    await this.world.api(
      `/api/conversations/${this.id}`,
      { target: workspaceId === null
          ? { kind: 'cloud' }
          : { kind: 'workspace', workspaceId } },
      'PATCH'
    )
    this.target = target
  }

  script(...turns: TurnScript[]): void {
    this.world.model.script(this.id, ...turns)
  }

  /**
   * Scripts the model for this turn, sends the user text, waits for the turn to
   * end.
   */
  async turn(script: {
    model: TurnScript[];
    text?: string;
    content?: Array<UserContentBlock | UploadBlock>
  }): Promise<Turn> {
    this.script(...script.model)
    const begin = this.begin()
    await this.client.send(
      (script.content ?? [{ type: 'text', text: script.text ?? 'go' }]) as UserContentBlock[]
    )
    return this.observe(begin)
  }

  /** Starts a turn without waiting for it: the detach and restart scenarios. */
  startTurn(script: {
    model: TurnScript[];
    text?: string
  }): {
    begin: TurnBegin;
    done: Promise<void>
  } {
    this.script(...script.model)
    const begin = this.begin()
    const done = this.client.send([{
        type: 'text',
        text: script.text ??
          'go'
      }])
      .catch(() => {})
    return { begin, done }
  }

  begin(): TurnBegin {
    return {
      events: this.events.length,
      blocks: this.transcript().length,
      requests: this.world.model.requests.length
    }
  }

  /**
   * The observation of a turn that began at `begin`; `received` covers tool
   * results not observed before.
   */
  observe(begin: TurnBegin): Turn {
    const requests = this.world.model.requests.slice(begin.requests)
      .filter((request) => request.sessionId === this.id)
    const received: string[] = []
    for (const request of requests) {
      for (const item of request.items) {
        if (item.type !== 'tool_result' ||
          this.seenToolResults.has(item.toolUseId))
          continue
        this.seenToolResults.add(item.toolUseId)
        received.push(
          item.output.map(
            (block) => (block.type === 'text' ? block.text : `[${block.type}]`)
          )
            .join('\n')
        )
      }
    }
    return {
      received,
      shell: this.events.slice(begin.events).filter(
        (event): event is ShellOutputEvent => event.type === 'shell_output'
      ),
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
  /**
   * Where a runner keeps a file under the conversation's home; a cloud
   * conversation's files are rows, read with `readFile`.
   */
  filePath(relative: string): string {
    if (this.target === 'cloud')
      throw new Error('a cloud conversation has no file path; use readFile')
    return join(
      this.world.device(this.target.slice('runner:'.length)).home,
      relative
    )
  }

  /**
   * A file under the conversation's home as the current target keeps it, or
   * null when absent.
   */
  async readFile(relative: string): Promise<string | null> {
    if (this.target === 'cloud')
      return this.world.cloudFile(this.id, relative)
    return readFile(this.filePath(relative), 'utf8').catch(() => null)
  }

  get agent(): AgentClient {
    return this.client
  }

  /**
   * Uploads bytes as an attachment and returns the block a `turn` sends to
   * carry it; the file lands at `attachmentPath(name)` on the host when the
   * message goes out (`product.md` § Attachments).
   */
  async upload(name: string, bytes: Uint8Array): Promise<UploadBlock> {
    const response = await this.world.backend.session.fetch(
      '/api/attachments',
      {
        method: 'POST',
        body: bytes,
        headers: { 'content-type': 'application/octet-stream' },
      }
    )
    if (response.status !== 201)
      throw new Error(
        `upload ${name}: HTTP ${response.status} ${await response.text()}`
      )
    const { attachment } = (await response.json()) as { attachment: { id: string } }
    return { type: 'upload', ref: attachment.id, fileName: name }
  }

  /** Where a sent attachment sits on the host, for a shell to read. */
  attachmentPath(name: string): string {
    return `~/${ATTACHMENTS_DIR}/${this.id}/${name}`
  }
}

/** The wire form of an upload inside a send frame (`attachment-refs.ts`). */
export type UploadBlock = z.infer<typeof uploadRefBlockSchema>

export interface TurnBegin {
  events: number
  blocks: number
  requests: number
}

/**
 * The allowed differences between the two targets, keyed by what a scenario
 * asks about.
 */
export function expected(target: Target) {
  return {
    hostCurrent: target === 'cloud'
      ? 'host: machine "Cloud"'
      : `on device "${target.slice('runner:'.length)}"`,
    gapNote: 'bytes not shown; the full stream is at',
    binaryKept: 'the raw bytes remain readable at',
    binaryPlaceholder: '; raw bytes at ',
    previewTruncated: 'previewTruncated: true; read ',
  }
}

/**
 * Turn scripts shaped as a real provider answers: every request ends with a
 * `response` carrying usage, so each one lands in the ledger.
 */
export const model = {
  /** A request answered with one shell_exec call. */
  shell: (
    toolUseId: string,
    script: string,
    timeoutMs = 10_000
  ): ProviderEvent[] => [
    events.toolCall(toolUseId, 'shell_exec', { script, timeoutMs }),
    events.response(),
  ],
  /** A request answered with one tool call of any kind. */
  tool: (
    toolUseId: string,
    toolName: string,
    input: unknown
  ): ProviderEvent[] => [
    events.toolCall(toolUseId, toolName, input),
    events.response()
  ],
  /** A request answered with text; the turn ends. */
  say: (text: string): ProviderEvent[] => [events.text(text), events.response()],
  /**
   * A request answered with text after a pause: a slow model, or a subagent
   * child that takes a while.
   */
  slowSay:
  (text: string, ms: number): TurnScript =>
  () =>
  (async function* () {
    await delay(ms)
    yield events.text(text)
    yield events.response()
  })(),
}
