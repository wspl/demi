import { createId, decodeUtf8, errorMessage, noop, utf8Slice } from '@demicodes/utils'
import { type ShellEnvironment, CommandRegistry, type Command, type CommandGroup, type CommandIO, type Host, type HostStore, RESERVED_COMMAND_NAMES, type ShellEnvironmentOptions } from '@demicodes/shell'
import type { Block, UserContentBlock } from '@demicodes/core'
import { AgentSession } from '../session/session'
import type { ServerFrame, SubagentJob, TranscriptPatch } from '../protocol/frames'
import type {
  AgentHarness,
  AgentHarnessRuntime,
  AgentMetadata,
  AgentSessionCheckpoint,
  AgentSessionStore,
  AgentToolInvokeContext,
  SubagentProfile,
} from '../types'
import { createStandardAgentTools } from '../tools'
import { hostAgentSessionStore } from '../store/session-store'
import type { BlobStore } from '../store/media'
import type { AgentServerSessionOptions, ShellEnvironmentFactory } from '../server/server'
import { childAgentNode, injectSubagentCommand, subagentCommandNode, subagentCommandShape } from './commands'
import { formatDuration } from './format'

export { injectSubagentCommand, subagentCommandShape }

export const MAX_LIVE_SUBAGENTS = 8
export const MAX_ARCHIVED_SUBAGENTS = 16
export const SUBAGENT_RESULT_MAX_BYTES = 32 * 1024
const SHOW_RECENT_TOOLS = 8

export type SubagentExecution =
  | 'idle'
  | 'provider_streaming'
  | 'tool_executing'
  | 'compacting'
  | 'finalizing'
  | 'pending_yield'

interface SubagentClose {
  phase: 'completed' | 'aborted' | 'error'
  result?: string
  failure?: string
}

interface ChildToolRecord {
  toolUseId: string
  title: string
  startedAt: number
  endedAt: number | null
  status: 'executing' | 'completed' | 'error'
}

interface ChildJob<State> {
  id: string
  description: string
  profile: SubagentProfile<State>
  profileName: string | null
  metadata: AgentMetadata | null
  session: AgentSession<State>
  commandRegistry: CommandRegistry
  commandNames: string[]
  environments: Map<Host, ShellEnvironment>
  pendingEnvironments: Map<Host, Promise<ShellEnvironment>>
  spawnedAt: number
  lastEventAt: number
  tools: ChildToolRecord[]
  lastAssistantTextAt: number | null
  phase: SubagentJob['phase']
  failure: string | null
  unsubscribe: () => void
  closed: Promise<SubagentClose>
  settleClosed: (close: SubagentClose) => void
  isClosing: boolean
}

export interface ChildSupervisorOptions<State> {
  agent: AgentHarness<State>
  /** The root session: children resolve their Host as it, since a child shares its parent's execution target. */
  agentSessionId: string
  cwd: string
  /** Harness-configured profiles; null means the implicit `default` only. */
  profiles: SubagentProfile<State>[] | null
  /** Parent registered commands (harness list, before the `demi agent` injection). */
  parentCommands: Command[]
  shellOptions: ShellEnvironmentOptions
  shellEnvironment: ShellEnvironmentFactory
  sessionOptions: AgentServerSessionOptions
  /** When false, a child closing never wakes an idle parent; the host app orchestrates the wakeup from the `subagent closed` frame. */
  notifyParentOnIdle: boolean
  /** Backing store for child session rows and job metadata, keyed under the parent's session directory. */
  store: HostStore
  /** Media blob store for child session persistence. */
  blobs?: BlobStore
  emit(frame: ServerFrame): void
}

/**
 * The on-store shape of a child (`.../subagents/<id>/job.json`); its checkpoint
 * sits beside it. Without `closedPhase` the child is live (a parent restore
 * resumes it); with `closedPhase` it is archived — finished but revivable with
 * `demi agent resume`, skipped by restore, pruned oldest-first past the cap.
 */
interface PersistedSubagentJob {
  description: string
  profileName: string | null
  metadata: AgentMetadata | null
  spawnedAt: number
  closedPhase?: SubagentClose['phase']
  closedAt?: number
}

/**
 * Per-parent subagent supervisor. Owns every child `AgentSession` this parent
 * spawns: lifecycle (spawn / steer / abort / natural end), the `demi agent`
 * command tree, child shell environments, and the `subagent*` protocol frames
 * on the parent connection. Children persist exactly like the parent: each
 * live child keeps a checkpoint and job record under the parent's session
 * directory, and reopening the parent restores and resumes them (`restore`).
 * A closed child moves to the archive: its transcript checkpoint stays on
 * store and `demi agent resume` revives it with a new user message.
 */
export class ChildSupervisor<State = unknown> {
  private readonly options: ChildSupervisorOptions<State>
  private readonly jobs = new Map<string, ChildJob<State>>()
  private parentSession: AgentSession<State> | null = null
  private isDisposed = false

  constructor(options: ChildSupervisorOptions<State>) {
    this.options = options
  }

  attachParent(session: AgentSession<State>): void {
    this.parentSession = session
  }

  /** The `agent` node AgentServer grafts under the parent registry's `demi` root. */
  rootCommandNode(): CommandGroup {
    return subagentCommandNode<ChildJob<State>>({
      profileNames: () => this.configuredProfileNames(),
      spawn: (input) => this.spawn(input),
      resumeArchived: (id, message) => this.resumeArchived(id, message),
      attend: (job, ctx) => this.attendChild(job, ctx),
      getRunning: (id) => this.jobs.get(id) ?? null,
      steer: (job, message) => this.steerChild(job, message),
      abortSubtree: (id) => this.abortSubtree(id),
      runningJobs: () => [...this.jobs.values()],
      listArchived: async () =>
        (await this.listArchivedJobs()).map(({ id, meta }) => ({
          id,
          description: meta.description,
          profileName: meta.profileName,
          closedPhase: meta.closedPhase,
          closedAt: meta.closedAt,
        })),
      snapshot: (job, detailed) => this.snapshot(job, detailed),
      renderListLine: (job) => this.renderListLine(job),
      renderShow: (job) => this.renderShow(job),
    })
  }

  hasShell(shellId: string): boolean {
    return this.environmentScopeForShell(shellId) !== null
  }

  /** Resolves the child scope owning a shell. */
  environmentScopeForShell(
    shellId: string,
  ): { environment: ShellEnvironment; commandNames: ReadonlySet<string>; agentSessionId: string } | null {
    for (const job of this.jobs.values()) {
      for (const environment of job.environments.values()) {
        if (environment.getShell(shellId)) {
          return { environment, commandNames: new Set(job.commandNames), agentSessionId: job.id }
        }
      }
    }
    return null
  }

  /** Re-emits `subagent started` + transcript reset for every running child (transcript resync). */
  replay(): void {
    for (const job of this.jobs.values()) {
      this.options.emit({ type: 'subagent', event: 'started', job: this.wireJob(job) })
      const transcript = job.session.transcript()
      this.options.emit({
        type: 'subagent_transcript_reset',
        subagentId: job.id,
        blocks: structuredClone(transcript.blocks),
        revision: transcript.revision,
      })
    }
  }

  /**
   * Detaches every live child on connection teardown: aborts in-flight turns,
   * flushes checkpoints, and keeps the persisted job so the next open of this
   * parent restores it — the same dispose semantics as the parent session.
   * No `closed` frame is emitted: the children are not done, just paused.
   */
  async dispose(): Promise<void> {
    this.isDisposed = true
    for (const job of [...this.jobs.values()]) {
      job.isClosing = true
      job.unsubscribe()
      this.jobs.delete(job.id)
      await job.session.dispose().catch(noop)
      await this.disposeJobShells(job)
      job.settleClosed({ phase: 'aborted' })
    }
  }

  /**
   * Rebuilds every persisted live child of this parent and finishes what it was
   * doing: an interrupted turn resumes from its resume point; an already
   * quiescent child closes with its result. Children share the parent's
   * persistence lifecycle — a parent restore is a child restore.
   */
  async restore(): Promise<void> {
    const parent = this.parentSession
    if (!parent || this.isDisposed) return
    const prefix = `agent-sessions/${parent.id()}/subagents/`
    const keys = await this.options.store.list(prefix).catch(() => [] as string[])
    const ids = [...new Set(keys.map((key) => key.slice(prefix.length).split('/')[0] ?? '').filter(Boolean))]
    for (const id of ids) {
      if (this.jobs.has(id)) continue
      try {
        await this.restoreJob(id)
      } catch {
        // A half-written or profile-orphaned child cannot be rebuilt; drop its remains.
        await this.deletePersistedJob(id)
      }
    }
  }

  private async restoreJob(id: string): Promise<void> {
    const parent = this.parentSession
    if (!parent) return
    const meta = await this.options.store.readJson<PersistedSubagentJob>(this.childStoreKey(id, 'job.json'))
    // Archived children are finished: only `demi agent resume` revives them.
    if (meta?.closedPhase) return
    const checkpoint = await this.childSessionStore(id).load()
    if (!meta || !checkpoint) throw new Error('incomplete persisted subagent')
    const job = this.reassembleJob(id, meta, checkpoint)
    // A live persisted job is by definition unfinished (closeJob archives it),
    // so always resume: findResumePoint decides how far to unwind, exactly like
    // an interrupted parent session. A child that had already produced its final
    // text re-infers one turn and closes through the normal quiescence path.
    void this.watchTurn(job, job.session.resume(meta.metadata ? { metadata: meta.metadata } : {}))
  }

  /** Shared by restore and resume: rebuild a persisted child's job and session from its checkpoint. */
  private reassembleJob(id: string, meta: PersistedSubagentJob, checkpoint: AgentSessionCheckpoint<State>): ChildJob<State> {
    const parent = this.parentSession
    if (!parent) throw new Error('subagent supervisor has no parent session')
    const profile = this.resolveProfile(meta.profileName ?? undefined)
    const { job, runtime } = this.assembleJob({
      id,
      description: meta.description,
      profileName: meta.profileName,
      profile,
      metadata: meta.metadata,
      spawnedAt: meta.spawnedAt,
    })
    const session = AgentSession.fromCheckpoint<State>(
      { provider: parent.cloneProviderRuntime(), runtime, checkpoint },
      { agentSessionId: id, store: this.childSessionStore(id), ...this.options.sessionOptions },
    )
    this.attachSession(job, session)
    return job
  }

  /**
   * Revives an archived child: its job record turns live again (this round's
   * metadata, a fresh spawnedAt), the session rebuilds from the preserved
   * checkpoint, and the message opens its next turn on top of the old
   * transcript.
   */
  private async resumeArchived(id: string, message: string): Promise<ChildJob<State>> {
    const parent = this.parentSession
    if (!parent) throw new Error('subagent supervisor has no parent session')
    if (this.isDisposed) throw new Error('parent session is closing')
    if (this.jobs.has(id)) throw new Error(`subagent "${id}" is still running; steer it instead`)
    if (this.jobs.size >= MAX_LIVE_SUBAGENTS) {
      throw new Error(`at most ${MAX_LIVE_SUBAGENTS} running subagents per session; abort one or wait for a result`)
    }
    const meta = await this.options.store.readJson<PersistedSubagentJob>(this.childStoreKey(id, 'job.json'))
    if (!meta?.closedPhase) throw new Error(`no archived subagent "${id}" (see \`demi agent list\`)`)
    const checkpoint = await this.childSessionStore(id).load()
    if (!checkpoint) throw new Error(`archived subagent "${id}" has no checkpoint left`)
    const liveMeta: PersistedSubagentJob = {
      description: meta.description,
      profileName: meta.profileName,
      metadata: parent.actionMetadata(),
      spawnedAt: Date.now(),
    }
    await this.options.store.writeJson(this.childStoreKey(id, 'job.json'), liveMeta)
    const job = this.reassembleJob(id, liveMeta, checkpoint)
    void this.watchTurn(job, job.session.send([{ type: 'text', text: message }], liveMeta.metadata ? { metadata: liveMeta.metadata } : {}))
    return job
  }

  /** Every archived (finished, revivable) child of this parent, newest first. */
  private async listArchivedJobs(): Promise<{ id: string; meta: PersistedSubagentJob }[]> {
    const parent = this.parentSession
    if (!parent) return []
    const prefix = `agent-sessions/${parent.id()}/subagents/`
    const keys = await this.options.store.list(prefix).catch(() => [] as string[])
    const ids = [...new Set(keys.map((key) => key.slice(prefix.length).split('/')[0] ?? '').filter(Boolean))]
    const archived: { id: string; meta: PersistedSubagentJob }[] = []
    for (const id of ids) {
      if (this.jobs.has(id)) continue
      const meta = await this.options.store.readJson<PersistedSubagentJob>(this.childStoreKey(id, 'job.json'))
      if (meta?.closedPhase) archived.push({ id, meta })
    }
    return archived.sort((a, b) => (b.meta.closedAt ?? 0) - (a.meta.closedAt ?? 0))
  }

  private async pruneArchive(): Promise<void> {
    const archived = await this.listArchivedJobs()
    for (const stale of archived.slice(MAX_ARCHIVED_SUBAGENTS)) {
      await this.deletePersistedJob(stale.id)
    }
  }

  async abortSubtree(id: string): Promise<void> {
    const job = this.jobs.get(id)
    if (!job) return
    // Depth is 1: the subtree is the node itself. Descendants would close here first.
    await this.closeJob(job, 'aborted')
  }

  /** Aborts every live child; the archive is untouched. */
  async abortAll(): Promise<void> {
    for (const id of [...this.jobs.keys()]) await this.abortSubtree(id)
  }

  private async spawn(input: {
    prompt: string
    profileName: string | undefined
    description: string
  }): Promise<ChildJob<State>> {
    const parent = this.parentSession
    if (!parent) throw new Error('subagent supervisor has no parent session')
    if (this.isDisposed) throw new Error('parent session is closing')
    if (this.jobs.size >= MAX_LIVE_SUBAGENTS) {
      throw new Error(`at most ${MAX_LIVE_SUBAGENTS} running subagents per session; abort one or wait for a result`)
    }
    const profile = this.resolveProfile(input.profileName)
    const id = createId()
    const metadata = parent.actionMetadata()
    const profileName = input.profileName ?? (this.options.profiles ? profile.name : null)
    const spawnedAt = Date.now()

    const { job, runtime } = this.assembleJob({
      id,
      description: input.description,
      profileName,
      profile,
      metadata,
      spawnedAt,
    })
    await this.options.store.writeJson<PersistedSubagentJob>(this.childStoreKey(id, 'job.json'), {
      description: input.description,
      profileName,
      metadata,
      spawnedAt,
    })
    const session = new AgentSession<State>(
      {
        provider: parent.cloneProviderRuntime(),
        model: profile.model ?? structuredClone(parent.modelSelection),
        cwd: this.options.cwd,
        runtime,
        state: this.options.agent.initialState(),
      },
      { agentSessionId: id, store: this.childSessionStore(id), ...this.options.sessionOptions },
    )
    this.attachSession(job, session)
    this.watchChild(job, input.prompt)
    return job
  }

  /** Everything spawn and restore share: the command tree, the harness runtime, and the job record (session attached separately). */
  private assembleJob(input: {
    id: string
    description: string
    profileName: string | null
    profile: SubagentProfile<State>
    metadata: AgentMetadata | null
    spawnedAt: number
  }): { job: ChildJob<State>; runtime: AgentHarnessRuntime<State> } {
    const { id, profile } = input
    const agentNode = this.createChildAgentNode(id, input.description)
    const inherited = profile.commands ? profile.commands([...this.options.parentCommands]) : [...this.options.parentCommands]
    const commands = injectSubagentCommand(inherited, agentNode)
    const commandRegistry = new CommandRegistry(RESERVED_COMMAND_NAMES)
    for (const command of commands) commandRegistry.register(command)
    const commandNames = commandRegistry.list().map((command) => command.name)
    const commandsPrompt = commandRegistry.renderHelp()

    let settleClosed!: (close: SubagentClose) => void
    const closed = new Promise<SubagentClose>((resolve) => {
      settleClosed = resolve
    })

    const job: ChildJob<State> = {
      id,
      description: input.description,
      profile,
      profileName: input.profileName,
      metadata: input.metadata,
      session: null as unknown as AgentSession<State>,
      commandRegistry,
      commandNames,
      environments: new Map(),
      pendingEnvironments: new Map(),
      spawnedAt: input.spawnedAt,
      lastEventAt: Date.now(),
      tools: [],
      lastAssistantTextAt: null,
      phase: 'running',
      failure: null,
      unsubscribe: noop,
      closed,
      settleClosed,
      isClosing: false,
    }

    const agent = this.options.agent
    const preamble = this.subagentPreamble(id)
    const runtime: AgentHarnessRuntime<State> = {
      harnessName: agent.name,
      initialState: () => agent.initialState(),
      systemPrompt: (ctx) => (profile.systemPrompt ?? agent.systemPrompt).call(agent, { ...ctx, commandsPrompt }),
      preamble: async (ctx) => {
        const inheritedPreamble = profile.systemPrompt ? null : ((await agent.preamble?.(ctx)) ?? null)
        return inheritedPreamble ? `${inheritedPreamble}\n\n${preamble}` : preamble
      },
      tools: () =>
        createStandardAgentTools<State>({
          environment: (ctx) => this.childEnvironment(job, ctx),
          scheduleYield: (ctx, durationMs) => job.session.scheduleYieldWakeup(durationMs, ctx.metadata),
        }),
    }
    return { job, runtime }
  }

  private attachSession(job: ChildJob<State>, session: AgentSession<State>): void {
    job.session = session
    job.unsubscribe = session.subscribe((event) => {
      if (event.type !== 'transcript_changed') return
      this.recordTelemetry(job, event.patches)
      this.options.emit({
        type: 'subagent_transcript_patch',
        subagentId: job.id,
        patches: event.patches,
        revision: event.revision,
      })
    })
    this.jobs.set(job.id, job)
    this.options.emit({ type: 'subagent', event: 'started', job: this.wireJob(job) })
    const transcript = session.transcript()
    this.options.emit({
      type: 'subagent_transcript_reset',
      subagentId: job.id,
      blocks: structuredClone(transcript.blocks),
      revision: transcript.revision,
    })
  }

  private childStoreKey(childId: string, file: string): string {
    return `agent-sessions/${this.parentSession?.id() ?? ''}/subagents/${childId}/${file}`
  }

  private childSessionStore(childId: string): AgentSessionStore<State> {
    return hostAgentSessionStore<State>(this.options.store, this.childStorePrefix(childId), {
      blobs: this.options.blobs,
    })
  }

  private childStorePrefix(childId: string): string {
    return `agent-sessions/${this.parentSession?.id() ?? ''}/subagents/${childId}`
  }

  private async deletePersistedJob(id: string): Promise<void> {
    for (const key of await this.options.store.list(`${this.childStorePrefix(id)}/`).catch(() => [] as string[])) {
      await this.options.store.delete(key).catch(noop)
    }
  }

  /** Shared spawn/resume foreground behaviour: announce the id, wire abort and stdin steers, wait for close, report the outcome. */
  private async attendChild(
    job: ChildJob<State>,
    ctx: { io: CommandIO; isJson: boolean; signal: AbortSignal; stdinStream: AsyncIterable<Uint8Array> },
  ): Promise<{ exitCode: number }> {
    const { io, isJson, signal, stdinStream } = ctx
    await io.stderr(`subagentId: ${job.id}\n`)

    const onAbort = (): void => {
      void this.abortSubtree(job.id).catch(noop)
    }
    if (signal.aborted) onAbort()
    else signal.addEventListener('abort', onAbort, { once: true })
    void this.pumpStdinSteers(job.id, stdinStream)

    const close = await job.closed
    signal.removeEventListener('abort', onAbort)
    if (close.phase === 'completed') {
      const text = close.result ?? ''
      if (isJson) await io.stdout(`${JSON.stringify({ subagentId: job.id, text })}\n`)
      else if (text.length > 0) await io.stdout(text.endsWith('\n') ? text : `${text}\n`)
      return { exitCode: 0 }
    }
    if (close.phase === 'aborted') {
      await io.stderr(`demi agent: subagent ${job.id} aborted\n`)
      return { exitCode: 130 }
    }
    await io.stderr(`demi agent: subagent ${job.id} failed: ${close.failure ?? 'unknown error'}\n`)
    return { exitCode: 1 }
  }

  /** Delivers one steer to a child: mid-turn as a steer, idle as a new user turn. */
  private async steerChild(job: ChildJob<State>, message: string): Promise<void> {
    const content: UserContentBlock[] = [{ type: 'text', text: message }]
    if (job.session.phase() !== 'idle') {
      try {
        await job.session.steer(content)
        return
      } catch {
        // Turn boundary raced the steer; fall through to a fresh turn.
      }
    }
    void this.watchTurn(job, job.session.send(content))
  }

  private async pumpStdinSteers(id: string, stdinStream: AsyncIterable<Uint8Array>): Promise<void> {
    try {
      for await (const chunk of stdinStream) {
        const job = this.jobs.get(id)
        if (!job) return
        const message = decodeUtf8(chunk).trim()
        if (message) await this.steerChild(job, message)
      }
    } catch {
      // The spawn command result reports the child outcome; stdin pump errors are not a channel.
    }
  }

  private watchChild(job: ChildJob<State>, prompt: string): void {
    const opening = job.session.send(
      [{ type: 'text', text: prompt }],
      job.metadata ? { metadata: job.metadata } : {},
    )
    void this.watchTurn(job, opening)
  }

  /** Tracks one child turn to session quiescence, then closes the job with its outcome. */
  private async watchTurn(job: ChildJob<State>, turn: Promise<void>): Promise<void> {
    try {
      await turn
    } catch (error) {
      job.failure = errorMessage(error)
      await this.closeJob(job, 'error')
      return
    }
    await this.waitForQuiescence(job)
    if (job.phase === 'running' && !job.isClosing) await this.closeJob(job, 'completed')
  }

  /** Resolves when the child is idle with no pending yield wakeups (or the job closed). */
  private async waitForQuiescence(job: ChildJob<State>): Promise<void> {
    while (!job.isClosing && (job.session.phase() !== 'idle' || job.session.hasPendingYields())) {
      await new Promise<void>((resolve) => {
        let settled = false
        const finish = (): void => {
          if (settled) return
          settled = true
          unsubscribe()
          resolve()
        }
        const unsubscribe = job.session.subscribe((event) => {
          if (event.type === 'phase_changed') finish()
        })
        void job.closed.then(finish)
      })
    }
  }

  private async closeJob(job: ChildJob<State>, phase: SubagentClose['phase']): Promise<void> {
    if (job.isClosing) return
    job.isClosing = true
    job.phase = phase
    job.unsubscribe()
    this.jobs.delete(job.id)

    const result = phase === 'completed' ? boundedResultText(lastAssistantText(job.session.transcript().blocks)) : undefined
    // dispose() flushes the final checkpoint; the archived job record beside it
    // marks the child finished-but-revivable.
    await job.session.dispose().catch(noop)
    await this.disposeJobShells(job)
    await this.options.store
      .writeJson<PersistedSubagentJob>(this.childStoreKey(job.id, 'job.json'), {
        description: job.description,
        profileName: job.profileName,
        metadata: job.metadata,
        spawnedAt: job.spawnedAt,
        closedPhase: phase,
        closedAt: Date.now(),
      })
      .catch(noop)
    await this.pruneArchive().catch(noop)

    const close: SubagentClose = {
      phase,
      ...(result !== undefined ? { result } : {}),
      ...(job.failure ? { failure: job.failure } : {}),
    }
    this.options.emit({ type: 'subagent', event: 'closed', job: this.wireJob(job, result) })
    job.settleClosed(close)
    this.notifyIdleParent(job, close)
  }

  private async disposeJobShells(job: ChildJob<State>): Promise<void> {
    const environments = new Set([...job.environments.values()])
    for (const pending of job.pendingEnvironments.values()) {
      const environment = await pending.catch(() => null)
      if (environment) environments.add(environment)
    }
    for (const environment of environments) await environment.disposeAllShells().catch(noop)
  }

  /** Wakes an idle parent with a user send; a parent blocked in the spawn gets the tool result instead. */
  private notifyIdleParent(job: ChildJob<State>, close: SubagentClose): void {
    if (this.isDisposed || !this.options.notifyParentOnIdle) return
    const parent = this.parentSession
    if (!parent || parent.phase() !== 'idle') return
    const label = `subagent ${job.id}${job.description ? ` — ${job.description}` : ''}`
    const body =
      close.phase === 'completed'
        ? `[${label}] completed.\nResult:\n${close.result || '(empty)'}`
        : close.phase === 'aborted'
          ? `[${label}] aborted.`
          : `[${label}] failed: ${close.failure ?? 'unknown error'}`
    // The wakeup round runs on behalf of the round that spawned the child, so it carries that round's metadata.
    void parent.send([{ type: 'text', text: body }], job.metadata ? { metadata: job.metadata } : {}).catch(noop)
  }

  private createChildAgentNode(childId: string, description: string): CommandGroup {
    return childAgentNode((message) => this.deliverToParent(childId, description, message))
  }

  private deliverToParent(childId: string, description: string, message: string): void {
    const parent = this.parentSession
    if (!parent || this.isDisposed) return
    const text = `[subagent ${childId}${description ? ` — ${description}` : ''}] ${message}`
    const content: UserContentBlock[] = [{ type: 'text', text }]
    const metadata = this.jobs.get(childId)?.metadata ?? null
    const sendOptions = metadata ? { metadata } : {}
    if (parent.phase() !== 'idle') {
      void parent.steer(content).catch(() => {
        void parent.send(content, sendOptions).catch(noop)
      })
      return
    }
    void parent.send(content, sendOptions).catch(noop)
  }

  private async childEnvironment(
    job: ChildJob<State>,
    ctx: Pick<AgentToolInvokeContext<State>, 'state' | 'metadata'>,
  ): Promise<ShellEnvironment> {
    const resolved = await this.options.agent.host({
      agentSessionId: this.options.agentSessionId,
      state: ctx.state,
      cwd: this.options.cwd,
      metadata: ctx.metadata,
    })
    const host = resolved
    const existing = job.environments.get(host)
    if (existing) return existing
    const pending = job.pendingEnvironments.get(host)
    if (pending) return pending
    const creation = this.createChildEnvironment(job, host)
    job.pendingEnvironments.set(host, creation)
    try {
      const environment = await creation
      job.environments.set(host, environment)
      return environment
    } finally {
      job.pendingEnvironments.delete(host)
    }
  }

  private async createChildEnvironment(job: ChildJob<State>, host: Host): Promise<ShellEnvironment> {
    const prepared = this.options.shellOptions
    return this.options.shellEnvironment({
      agentSessionId: job.id,
      host,
      commands: job.commandRegistry,
      shell: {
        ...prepared,
        initialEnv: {
          ...prepared.initialEnv,
          DEMI_SUBAGENT_ID: job.id,
          DEMI_PARENT_SESSION_ID: this.parentSession?.id() ?? '',
          DEMI_SUBAGENT_DEPTH: '1',
        },
      },
    })
  }

  private resolveProfile(name: string | undefined): SubagentProfile<State> {
    const profiles = this.options.profiles
    const implicitDefault: SubagentProfile<State> = {
      name: 'default',
      description: 'Inherits the parent harness, model, Host, and commands.',
    }
    if (!profiles || profiles.length === 0) {
      if (name !== undefined && name !== 'default') {
        throw new Error(`unknown profile "${name}" (available: default)`)
      }
      return implicitDefault
    }
    const target = name ?? 'default'
    const profile = profiles.find((candidate) => candidate.name === target)
    if (profile) return profile
    if (name === undefined) return implicitDefault
    throw new Error(`unknown profile "${name}" (available: ${this.configuredProfileNames().join(', ')})`)
  }

  private configuredProfileNames(): string[] {
    const names = (this.options.profiles ?? []).map((profile) => profile.name)
    return names.includes('default') ? names : ['default', ...names]
  }

  private subagentPreamble(childId: string): string {
    return [
      `You are a subagent: an isolated child agent session (id ${childId}) spawned by a parent agent session. Your transcript starts empty; the task brief in the first user message is your entire context.`,
      'When you end your turn with no scheduled wakeups, the session ends and your last assistant text is returned to the parent as the result. Write it for the parent agent, in the shape the task brief asked for.',
      '`demi agent send-parent <message>` sends an interim user message to the parent; it is seen only when the parent is not blocked waiting on this session.',
      'You are not talking to the product user; do not address them.',
    ].join('\n')
  }

  private recordTelemetry(job: ChildJob<State>, patches: TranscriptPatch[]): void {
    const now = Date.now()
    for (const patch of patches) {
      if (patch.op === 'add') {
        const block = patch.value
        if (block.type === 'tool_call') {
          job.tools.push({
            toolUseId: block.toolUseId,
            title: toolCallTitle(block),
            startedAt: now,
            endedAt: null,
            status: 'executing',
          })
          trimToolRecords(job.tools)
          job.lastEventAt = now
        } else if (block.type === 'text') {
          job.lastAssistantTextAt = now
          job.lastEventAt = now
        }
        continue
      }
      if (patch.op === 'append_text') {
        const block = job.session.transcript().blocks[patch.path[1]]
        if (block?.type === 'text') {
          job.lastAssistantTextAt = now
          job.lastEventAt = now
        }
        continue
      }
      if (patch.op === 'replace_block') {
        const block = patch.value
        if (block.type !== 'tool_call' || block.status === 'executing') continue
        const record = job.tools.find((tool) => tool.toolUseId === block.toolUseId && tool.endedAt === null)
        if (record) {
          record.endedAt = now
          record.status = block.status === 'error' ? 'error' : 'completed'
        }
        job.lastEventAt = now
      }
    }
  }

  private executionOf(job: ChildJob<State>): SubagentExecution {
    const phase = job.session.phase()
    if (phase === 'compacting') return 'compacting'
    if (phase === 'idle') return job.session.hasPendingYields() ? 'pending_yield' : 'idle'
    return job.session.turnPhase() ?? 'provider_streaming'
  }

  private activityOf(job: ChildJob<State>, execution: SubagentExecution): string {
    const inflight = [...job.tools].reverse().find((tool) => tool.endedAt === null)
    if (execution === 'tool_executing' && inflight) return inflight.title
    if (execution === 'provider_streaming') return 'streaming'
    return execution
  }

  private executionForMs(job: ChildJob<State>, execution: SubagentExecution, now: number): number {
    if (execution === 'tool_executing') {
      const inflight = [...job.tools].reverse().find((tool) => tool.endedAt === null)
      if (inflight) return now - inflight.startedAt
    }
    return now - job.lastEventAt
  }

  private snapshot(job: ChildJob<State>, detailed: boolean): Record<string, unknown> {
    const now = Date.now()
    const execution = this.executionOf(job)
    const base: Record<string, unknown> = {
      subagentId: job.id,
      description: job.description,
      profile: job.profileName,
      phase: job.phase,
      elapsedMs: now - job.spawnedAt,
      lastEventMs: now - job.lastEventAt,
      execution,
      activity: this.activityOf(job, execution),
    }
    if (!detailed) return base
    const text = lastAssistantText(job.session.transcript().blocks)
    return {
      ...base,
      executionForMs: this.executionForMs(job, execution, now),
      tools: job.tools.slice(-SHOW_RECENT_TOOLS).map((tool) => ({
        title: tool.title,
        status: tool.status,
        durationMs: (tool.endedAt ?? now) - tool.startedAt,
        endedAgoMs: tool.endedAt === null ? null : now - tool.endedAt,
      })),
      lastAssistantText: boundedResultText(text),
      lastAssistantTextAgoMs: job.lastAssistantTextAt === null ? null : now - job.lastAssistantTextAt,
    }
  }

  private renderListLine(job: ChildJob<State>): string {
    const now = Date.now()
    const execution = this.executionOf(job)
    const parts = [
      job.id,
      job.phase,
      `up ${formatDuration(now - job.spawnedAt)}`,
      `last-event ${formatDuration(now - job.lastEventAt)} ago`,
      `profile=${job.profileName ?? 'default'}`,
      job.description ? `"${job.description}"` : '(no description)',
      `execution=${execution}`,
      `activity=${this.activityOf(job, execution)}`,
    ]
    return parts.join('  ')
  }

  private renderShow(job: ChildJob<State>): string {
    const now = Date.now()
    const execution = this.executionOf(job)
    const lines = [
      `id: ${job.id}`,
      `description: ${job.description || '(none)'}`,
      `profile: ${job.profileName ?? 'default'}`,
      `phase: ${job.phase}`,
      `elapsed: ${formatDuration(now - job.spawnedAt)}`,
      `execution: ${execution} (for ${formatDuration(this.executionForMs(job, execution, now))})`,
      `last-event: ${formatDuration(now - job.lastEventAt)} ago`,
      `activity: ${this.activityOf(job, execution)}`,
    ]
    const recent = job.tools.slice(-SHOW_RECENT_TOOLS)
    if (recent.length > 0) {
      lines.push(`recent tool calls (last ${recent.length}):`)
      for (const tool of recent) {
        if (tool.endedAt === null) {
          lines.push(`  [executing for ${formatDuration(now - tool.startedAt)}] ${tool.title}`)
        } else {
          lines.push(
            `  [${tool.status} in ${formatDuration(tool.endedAt - tool.startedAt)}, ended ${formatDuration(now - tool.endedAt)} ago] ${tool.title}`,
          )
        }
      }
    }
    const text = boundedResultText(lastAssistantText(job.session.transcript().blocks))
    if (text && job.lastAssistantTextAt !== null) {
      lines.push(`last assistant text (${formatDuration(now - job.lastAssistantTextAt)} ago):`)
      lines.push(text)
    } else {
      lines.push('last assistant text: (none yet)')
    }
    return `${lines.join('\n')}\n`
  }

  private wireJob(job: ChildJob<State>, result?: string): SubagentJob {
    return {
      subagentId: job.id,
      parentSessionId: this.parentSession?.id() ?? '',
      description: job.description,
      profile: job.profileName,
      phase: job.phase,
      metadata: job.metadata,
      ...(result !== undefined ? { result } : {}),
    }
  }
}

function trimToolRecords(tools: ChildToolRecord[]): void {
  while (tools.length > SHOW_RECENT_TOOLS) {
    const index = tools.findIndex((tool) => tool.endedAt !== null)
    if (index === -1) return
    tools.splice(index, 1)
  }
}

function toolCallTitle(block: Extract<Block, { type: 'tool_call' }>): string {
  try {
    const input = JSON.parse(block.input) as Record<string, unknown>
    if (typeof input.description === 'string' && input.description.trim()) return input.description.trim()
  } catch {
    // Fall through to the tool name.
  }
  return block.toolName
}

function lastAssistantText(blocks: Block[]): string {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i]
    if (block.type === 'text') return block.text
  }
  return ''
}

function boundedResultText(text: string): string {
  return utf8Slice(text, 0, SUBAGENT_RESULT_MAX_BYTES)
}

