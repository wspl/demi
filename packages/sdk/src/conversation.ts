/**
 * A conversation of a core, driven as the web app drives one: a
 * `ConversationClient` on the conversation's lane, which reads with the
 * core's routes, holds the root's transcript with the one patch applier, and
 * sends what the program asks.
 *
 * `run(input)` sends a message and waits for the turn it starts to end, as
 * the Codex SDK's `thread.run`; `events()` is everything the conversation's
 * lane tells, as an async iterator, with a `permission` event for each
 * request the conversation's metadata adds.
 */
import {
  applyTranscriptPatches,
  ConversationClient,
  createConversationClient,
  EMPTY_TRANSCRIPT,
  heldBlocks,
  resetTranscript,
  type ClientSessionEvent,
  type HeldTranscript,
} from '@demicodes/conversation-client'
import {
  conversationDetailSchema,
  conversationFrameSchema,
  transcriptPageSchema,
  type AbortResult,
  type Block,
  type ClientContent,
  type ConversationMetadata,
  type PermissionRequest,
  type RequestedValues,
  type TurnOutcome,
} from '@demicodes/protocol'
import { randomUUID } from 'node:crypto'
import type { CoreRoutes } from './routes'

/** What a program sends: text, or the content blocks a page sends. */
export type Input = string | ClientContent[]

/** A turn that ended: how, the blocks it wrote after its message, and their text. */
export interface Turn {
  outcome: TurnOutcome
  blocks: Block[]
  text: string
}

/** A permission request of the conversation, which the program may decide. */
export interface Permission {
  readonly request: PermissionRequest
  /** Grants it; a credential's request takes the values asked for. */
  allow(values?: RequestedValues): Promise<void>
  deny(): Promise<void>
}

/** What `events()` yields: the lane's events, and each new permission request. */
export type ConversationEvent = ClientSessionEvent | { type: 'permission'; permission: Permission }

export class Conversation {
  private client: ConversationClient | null = null
  private opening: Promise<ConversationClient> | null = null
  /** The root's transcript, as the lane keeps it. */
  private transcript: HeldTranscript = EMPTY_TRANSCRIPT
  private metadata: ConversationMetadata | null = null
  /** The permission requests told already. */
  private readonly told = new Set<string>()
  private readonly listeners = new Set<(event: ConversationEvent) => void>()
  /** What waits for a change of the transcript or the metadata. */
  private readonly changes = new Set<() => void>()

  constructor(readonly id: string, private readonly routes: CoreRoutes) {}

  /** Opens the conversation's lane, once; each action opens it first. */
  open(): Promise<ConversationClient> {
    this.opening ??= this.connect()
    return this.opening
  }

  private async connect(): Promise<ConversationClient> {
    const path = `/conversations/${encodeURIComponent(this.id)}`
    const client = createConversationClient(this.routes.connection, this.id, {
      reader: {
        conversation: async (signal) => {
          const detail = await this.routes.json('GET', path, conversationDetailSchema, undefined, signal)
          this.takeMetadata(detail.metadata)
          return { agents: detail.agents, entries: detail.entries, tags: { metadata: detail.metadataTag } }
        },
        transcript: async (node, signal) => {
          const page = await this.routes.json('GET', `${path}/agents/${encodeURIComponent(node)}/transcript?latest=true`, transcriptPageSchema, undefined, signal)
          if (!page.version) {
            throw new Error('The latest page names no version of its transcript')
          }
          return { ...page, version: page.version }
        },
      },
      conversationFrame: (frame) => {
        const parsed = conversationFrameSchema.safeParse(frame)
        if (!parsed.success) {
          return null
        }
        const data = parsed.data
        if (data.type === 'conversation') {
          return { apply: () => this.takeMetadata(data.metadata), tagged: { value: 'metadata', tag: data.etag } }
        }
        return { apply: () => {} }
      },
    })
    client.subscribe((event) => this.receive(event))
    this.client = client
    await client.open({ root: this.id })
    return client
  }

  private receive(event: ClientSessionEvent): void {
    switch (event.type) {
      case 'transcript_reset':
        if (event.node === this.id) {
          this.transcript = resetTranscript(this.transcript, event, event.asked)
        }
        break
      case 'transcript_patch':
        if (event.node === this.id) {
          this.transcript = applyTranscriptPatches(this.transcript, event.patches, performance.now(), event.version)
        }
        break
    }
    this.emit(event)
    this.changed()
  }

  private takeMetadata(metadata: ConversationMetadata): void {
    this.metadata = metadata
    for (const request of metadata.permissions.requests) {
      if (!this.told.has(request.id)) {
        this.told.add(request.id)
        this.emit({ type: 'permission', permission: this.permission(request) })
      }
    }
    this.changed()
  }

  private changed(): void {
    for (const change of [...this.changes]) {
      change()
    }
  }

  private permission(request: PermissionRequest): Permission {
    const decide = (body: unknown) => this.routes.send('POST', `/conversations/${encodeURIComponent(this.id)}/permissions/requests/${encodeURIComponent(request.id)}`, body)
    return {
      request,
      allow: (values) => decide({ decision: 'allow', ...(values === undefined ? {} : { credential: values }) }),
      deny: () => decide({ decision: 'deny' }),
    }
  }

  private emit(event: ConversationEvent): void {
    for (const listener of [...this.listeners]) {
      listener(event)
    }
  }

  /** The permission requests waiting for a decision. */
  get permissions(): Permission[] {
    return (this.metadata?.permissions.requests ?? []).map((request) => this.permission(request))
  }

  /** The root's blocks the conversation holds, oldest first. */
  get blocks(): Block[] {
    return heldBlocks(this.transcript)
  }

  /** Sends a message; resolves once it is in the transcript or the queue. */
  async send(input: Input, messageId: string = randomUUID()): Promise<void> {
    const client = await this.open()
    await client.submit(content(input), messageId)
  }

  /** Steers the running turn with `input`; resolves once the turn took it. */
  async steer(input: Input): Promise<void> {
    const client = await this.open()
    await client.steer(content(input))
  }

  async abort(): Promise<AbortResult> {
    return (await this.open()).abort()
  }

  async retry(): Promise<void> {
    return (await this.open()).retry()
  }

  async resume(): Promise<void> {
    return (await this.open()).resume()
  }

  async compact(): Promise<void> {
    return (await this.open()).compact()
  }

  /**
   * Sends `input`, and answers the turn it starts once the turn ended: once
   * the conversation's last ended turn is one whose ending block follows
   * the message, as a turn the root resumes ends anew.
   */
  async run(input: Input): Promise<Turn> {
    const messageId = randomUUID()
    const ended = new Promise<Turn>((resolve) => {
      const check = () => {
        const turn = this.turnOf(messageId)
        if (turn !== null) {
          this.changes.delete(check)
          resolve(turn)
        }
      }
      this.changes.add(check)
    })
    await this.send(input, messageId)
    return ended
  }

  /** The turn `messageId` started, once it ended; null before. */
  private turnOf(messageId: string): Turn | null {
    const last = this.metadata?.lastTurn
    if (!last) {
      return null
    }
    const blocks = this.blocks
    const at = blocks.findIndex((block) => block.type === 'user' && block.turnId === messageId)
    const end = blocks.findIndex((block) => block.id === last.id)
    if (at < 0 || end <= at) {
      return null
    }
    const after = blocks.slice(at + 1, end + 1)
    const text = after.flatMap((block) => (block.type === 'text' ? [block.text] : [])).join('\n\n')
    return { outcome: last.outcome, blocks: after, text }
  }

  /** Everything the conversation's lane tells from now on, until the loop that reads it stops. */
  async *events(): AsyncGenerator<ConversationEvent, void, undefined> {
    const waiting: ConversationEvent[] = []
    let wake: (() => void) | null = null
    const listener = (event: ConversationEvent) => {
      waiting.push(event)
      wake?.()
    }
    this.listeners.add(listener)
    try {
      await this.open()
      for (;;) {
        const event = waiting.shift()
        if (event !== undefined) {
          yield event
          continue
        }
        await new Promise<void>((resolve) => (wake = resolve))
        wake = null
      }
    } finally {
      this.listeners.delete(listener)
    }
  }

  /** Closes the conversation's lane. */
  async close(): Promise<void> {
    const client = this.client
    this.client = null
    this.opening = null
    await client?.close()
  }
}

function content(input: Input): ClientContent[] {
  return typeof input === 'string' ? [{ type: 'text', text: input }] : input
}
