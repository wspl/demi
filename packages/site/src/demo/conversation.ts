// A demo's conversation: the state `ChatSession` reads, and the writes a
// scene makes to it over time, as the product's runtime would make them.
import { reactive } from 'vue'
import type { Block, ChangedFile, EditedFile, ModelSelection, UserContentBlock } from '@demicodes/protocol'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import type { ConversationFiles, MessageAttachment } from '@demicodes/web-ui/markdown/types'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'

export const PROVIDERS: ProviderInfo[] = [{ id: 'anthropic', label: 'Anthropic', isAvailable: true }]

const SONNET: ModelInfo = {
  id: 'claude-sonnet',
  name: 'Claude Sonnet',
  contextWindow: 200_000,
  contextLimit: null,
  acceptedExtensions: ['png', 'pdf', 'md'],
  reasoning: null,
  serviceTiers: null,
}

export const MODELS: Record<string, ModelInfo[]> = { anthropic: [SONNET] }

export const MODEL_SETTINGS = { providerId: 'anthropic', modelId: SONNET.id, thinkingEffort: null, serviceTierId: null }

export const MODEL: ModelSelection = {
  providerId: 'anthropic',
  model: { id: SONNET.id, name: SONNET.name, contextWindow: 200_000, outputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
  serviceTierId: null,
}

export type DemoState = ChatSessionState & { blocks: Block[] }

/** Thrown out of a scene's wait when the scene was replaced. */
export class Stopped extends Error {}

/** The blob a demo's file names: its bytes come from its URL, never the blob store. */
const NO_BLOB = '0'.repeat(64)

/** A file a scene's message or reply carries: an image the user sends, or a file the agent hands over. */
export interface DemoFile {
  name: string
  mediaType: string
  url: string
  size: number
  width?: number
  height?: number
}

/** A file a shell call writes, by its path in the conversation's working directory. */
export interface DemoEdit {
  path: string
  added: number
  removed: number
  /** The call creates the file. */
  created?: boolean
}

/** A shell call as a scene describes it. */
export interface ShellStep {
  description: string
  script: string
  output?: string
  /** 0 by default. */
  exitCode?: number
  /** The files the command writes, which its row's pills name. */
  files?: readonly DemoEdit[]
  /** How long the command runs. */
  ms?: number
}

/**
 * A conversation a scene writes to. Every write is one the runtime makes:
 * a message, the agent's thinking, its calls and its reply, each block in
 * its request and its run as the backend stores them.
 */
export class Conversation {
  readonly state: DemoState
  private serial = 0
  private stopped = false
  private readonly timers = new Set<ReturnType<typeof setTimeout>>()
  /** The conversation's attachments, by their numbers, `a1` first. */
  private readonly attachments = reactive(new Map<string, MessageAttachment>())
  /** What `ChatSession` reads of the conversation's files: its attachments. */
  readonly files: ConversationFiles = {
    attachment: (id) => {
      const attachment = this.attachments.get(id)
      return attachment ? { state: 'found', attachment } : { state: 'missing' }
    },
  }

  constructor(id: string, title: string, cwd: string) {
    this.state = reactive({
      id,
      cwd,
      title,
      blocks: [],
      get history() {
        return wholeHistory(this.blocks as never)
      },
      shownAt: null,
      summaries: {},
      queue: [],
      pendingSteers: [],
      starting: null,
      pendingCalls: [],
      waitingSince: null,
      requestOutputs: {},
      phase: 'idle',
      load: 'ready',
      lastError: null,
      pendingAction: null,
      failures: {},
      archived: false,
      scroll: null,
      subagents: [],
      terminals: [],
      turnSpeed: null,
    }) as DemoState
  }

  /** Ends the scene: every wait after this throws `Stopped`. */
  stop(): void {
    this.stopped = true
    for (const timer of this.timers)
      clearTimeout(timer)
    this.timers.clear()
  }

  wait(ms: number): Promise<void> {
    if (this.stopped)
      return Promise.reject(new Stopped())
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.timers.delete(timer)
        if (this.stopped)
          reject(new Stopped())
        else
          resolve()
      }, ms)
      this.timers.add(timer)
    })
  }

  /** Calls `act` in `ms`, beside what the scene goes on doing; nothing once the scene stopped. */
  after(ms: number, act: () => void): void {
    this.wait(ms).then(act, (error: unknown) => {
      if (!(error instanceof Stopped))
        throw error
    })
  }

  private id(kind: string): string {
    this.serial += 1
    return `${kind}-${this.serial}`
  }

  private now(offsetMs = 0): string {
    return new Date(Date.now() - offsetMs).toISOString()
  }

  /**
   * `block` at the end of the transcript, as the backend stores it: in the
   * request of the last message, and a step (thinking or a call) in the run
   * the step before it opened.
   */
  private append(block: Block): Block {
    const blocks = this.state.blocks
    const request = block.type === 'user' ? block.id : blocks.findLast((each) => each.type === 'user')?.id
    let placed = { ...block, request } as Block
    if (block.type === 'thinking' || block.type === 'tool_call') {
      const before = blocks.at(-1)
      const run = before && 'run' in before && before.run !== undefined ? before.run : block.id
      placed = { ...placed, run } as Block
    }
    this.state.blocks = [...blocks, placed]
    return placed
  }

  private replace(id: string, change: (block: Block) => Block): void {
    this.state.blocks = this.state.blocks.map((block) => (block.id === id ? change(block) : block))
  }

  /** A message already in the transcript when the scene opens, `agoMs` ago. */
  past(text: string, reply: string, agoMs: number): void {
    const user = this.userBlock([{ type: 'text', text }], agoMs)
    this.append(user)
    this.append({ type: 'text', id: this.id('text'), createdAt: this.now(agoMs - 4000), model: MODEL, text: reply, forkable: true } as Block)
  }

  private userBlock(content: UserContentBlock[], agoMs = 0): Block {
    return { type: 'user', id: this.id('user'), turnId: this.id('turn'), createdAt: this.now(agoMs), model: MODEL, content, preamble: null } as Block
  }

  /** Numbers `file` as the conversation's next attachment, as an upload does; answers its number. */
  attach(file: DemoFile): string {
    const id = `a${this.attachments.size + 1}`
    this.attachments.set(id, { name: file.name, mediaType: file.mediaType, url: file.url, size: file.size, sha256: NO_BLOB, ...(file.width ? { width: file.width, height: file.height } : {}) })
    return id
  }

  /** The user sent `text` `agoMs` ago, and the agent is at work on it when the scene opens. */
  asked(text: string, agoMs: number): void {
    this.append(this.userBlock([{ type: 'text', text }], agoMs))
    this.state.phase = 'running'
  }

  /** The user sends `text` with `images`: the turn starts and the agent is waited for. */
  async send(text: string, images: readonly DemoFile[] = []): Promise<void> {
    // Each image is the medium the model reads and its attachment's record, as an upload sends it.
    const files = images.flatMap((image): UserContentBlock[] => [
      { type: 'image', source: { type: 'url', url: image.url } },
      { type: 'attachment', id: this.attach(image), name: image.name, mediaType: image.mediaType, sizeBytes: image.size, sha256: NO_BLOB },
    ])
    this.append(this.userBlock([{ type: 'text', text }, ...files]))
    this.state.phase = 'running'
    this.state.waitingSince = Date.now()
    await this.wait(700)
  }

  /** The agent thinks `text`, a few words at a time. */
  async think(text: string, ms = 1200): Promise<void> {
    this.state.waitingSince = null
    const block = this.append({ type: 'thinking', id: this.id('thinking'), createdAt: this.now(), model: MODEL, text: '', signature: null } as Block)
    await this.write(block.id, text, ms)
  }

  /** The agent answers `text`, as it streams. */
  async say(text: string, ms = 1600): Promise<void> {
    this.state.waitingSince = null
    const units = segmentStreamUnits(text)
    // The backend opens a text block with its first words, never empty.
    const block = this.append({ type: 'text', id: this.id('text'), createdAt: this.now(), model: MODEL, text: units[0] ?? '' } as Block)
    await this.write(block.id, text, ms)
    // The response ends: the backend marks its text complete.
    this.replace(block.id, (each) => ({ ...each, forkable: true }) as Block)
  }

  private async write(id: string, text: string, ms: number): Promise<void> {
    const units = segmentStreamUnits(text)
    const step = Math.max(16, ms / Math.max(1, units.length))
    let shown = ''
    for (const unit of units) {
      shown += unit
      const now = shown
      this.replace(id, (block) => ({ ...block, text: now }) as Block)
      await this.wait(step)
    }
  }

  /** The agent runs a shell command, which prints its output and exits. */
  async shell(step: ShellStep): Promise<void> {
    this.state.waitingSince = null
    const ms = step.ms ?? 900
    const call = this.call(step)
    await this.wait(ms)
    this.exited(call, step, ms)
    this.state.waitingSince = Date.now()
    await this.wait(300)
  }

  /** A shell command the agent ran just before the scene opens, `step.ms` long. */
  ran(step: ShellStep): void {
    const ms = step.ms ?? 900
    this.exited(this.call(step, ms), step, ms)
  }

  /** The call of `step`, running since `agoMs` ago. */
  private call(step: ShellStep, agoMs = 0): ToolCallBlock {
    const createdAt = this.now(agoMs)
    const commandId = String(this.serial + 10)
    return this.append({
      type: 'tool_call',
      id: this.id('call'),
      createdAt,
      model: MODEL,
      toolUseId: this.id('use'),
      toolName: 'shell',
      input: JSON.stringify({ script: step.script, description: step.description }),
      status: 'executing',
      output: [],
      writingStartedAt: null,
      endedAt: null,
      view: { kind: 'shell', status: 'running', commandId, runningMs: 0, idleMs: 0, viewTruncated: false, chunks: [] },
    } as ToolCallBlock as Block) as ToolCallBlock
  }

  /** `call` exits, `ms` after it started, with what `step` prints and writes. */
  private exited(call: ToolCallBlock, step: ShellStep, ms: number): void {
    const { createdAt } = call
    const commandId = call.view?.kind === 'shell' ? call.view.commandId : ''
    const files = (step.files ?? []).map((file): EditedFile => ({
      path: `${this.state.cwd}/${file.path}`,
      kind: file.created ? 'added' : 'modified',
      added: file.added,
      removed: file.removed,
      edits: [{}],
    }))
    const runFiles = this.runFiles(call, commandId, files)
    this.replace(call.id, (block) => ({
      ...block,
      status: 'completed',
      endedAt: new Date(Date.parse(createdAt) + ms).toISOString(),
      view: {
        kind: 'shell',
        status: 'exited',
        exitCode: step.exitCode ?? 0,
        commandId,
        runningMs: ms,
        idleMs: 0,
        viewTruncated: false,
        chunks: step.output ? [{ stream: 'stdout', text: step.output }] : [],
        ...(files.length > 0 ? { files } : {}),
        ...(runFiles.length > 0 ? { runFiles } : {}),
      },
    }) as Block)
  }

  /**
   * The files `call`'s run has written once `files`, `command`'s, are
   * written, as the backend stores them on the run's latest call: each file
   * once, its lines counted across the run.
   */
  private runFiles(call: Block, command: string, files: readonly EditedFile[]): ChangedFile[] {
    const run = 'run' in call ? call.run : undefined
    const earlier = this.state.blocks
      .filter((block): block is ToolCallBlock => block.type === 'tool_call' && block.id !== call.id && 'run' in block && block.run === run)
      .map((block) => (block.view?.kind === 'shell' ? block.view.runFiles : undefined))
      .findLast((each) => each !== undefined) ?? []
    const byPath = new Map<string, ChangedFile>()
    for (const file of earlier)
      byPath.set(file.path, file)
    for (const file of files) {
      const before = byPath.get(file.path)
      const edit = { call: command, path: file.path, segment: 0 }
      byPath.set(file.path, before
        ? { ...before, lines: { added: (before.lines?.added ?? 0) + file.added, removed: (before.lines?.removed ?? 0) + file.removed }, edits: [...before.edits, edit] }
        : { path: file.path, kind: file.kind, lines: { added: file.added, removed: file.removed }, edits: [{ ...edit, ...(file.kind === 'added' ? { created: true } : {}) }] })
    }
    return [...byPath.values()]
  }

  /**
   * The user moved the conversation to `host`, where its work starts in
   * `path`: the agent hears of it, and the transcript says where it runs now.
   */
  moved(host: string, path: string, home: string): void {
    const at = this.now()
    this.append({
      type: 'agent_message',
      id: this.id('moved'),
      turnId: this.id('turn'),
      createdAt: at,
      model: MODEL,
      message: { id: `moved:${this.serial}`, recipientId: this.state.id, timestamp: at, content: '', event: { type: 'moved', host, path, home } },
    } as Block)
    this.state.cwd = path
  }

  /** The turn ends. */
  done(): void {
    this.state.waitingSince = null
    this.state.phase = 'idle'
  }
}
