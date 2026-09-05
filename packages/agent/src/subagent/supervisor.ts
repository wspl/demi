import { createId, decodeUtf8, errorMessage, noop, utf8Slice } from '@demicodes/utils'
import type { Command, CommandGroup, CommandIO, ShellEnvironment } from '@demicodes/shell'
import type { Block, QueuedMessage, UserContentBlock } from '@demicodes/core'
import { textContentSummary, type AgentSession } from '../session/session'
import type { ServerFrame, SubagentJob, TranscriptPatch } from '../protocol/frames'
import type { AgentHarness, AgentMetadata, AgentNodeClosePhase, AgentNodeRecord, SubagentProfile } from '../types'
import type { AssembledNode, NodeDeps, NodeParams, TreeContext } from '../node/assemble'
import { CHILD_POLICY, type SessionNode } from '../node/node'
import { completionMessageId } from '../store/tree-store'
import { injectSubagentCommand, subagentCommandNode, subagentCommandShape } from './commands'
import { formatDuration } from './format'

export { injectSubagentCommand, subagentCommandShape }

/** Default per-session live-children ceiling; override with `AgentServerOptions.subagents.maxLiveSubagents`. */
export const MAX_LIVE_SUBAGENTS = 8
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
  phase: AgentNodeClosePhase
  result?: string
  failure?: string
}

/** A closed child as its parent hears of it, live or from the store. */
interface Completion {
  id: string
  description: string
  metadata: AgentMetadata | null
  phase: AgentNodeClosePhase
  result: string | null
  failure: string | null
}

interface ChildToolRecord {
  toolUseId: string
  title: string
  startedAt: number
  endedAt: number | null
  status: 'executing' | 'completed' | 'error'
}

/** A live child as its supervisor sees it: the node plus the relationship's bookkeeping. */
export interface ChildJob<State> {
  id: string
  node: SessionNode<State>
  description: string
  profileName: string | null
  metadata: AgentMetadata | null
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
  /** True while the spawn or resume command awaits this child in this process: its exit is the result's return path. */
  attended: boolean
  /** Wakes the settle loop when the child set or an inbound send changes the picture. */
  wake: (() => void) | null
}

/** The reserved word a harness may not use as a profile name: it is not a profile, it is the absence of one. */
const INHERIT_PROFILE_NAME = 'default'
const INHERIT_PROFILE_LABEL = '(inherit)'
const INHERIT_PROFILE: SubagentProfile<unknown> = {
  name: INHERIT_PROFILE_LABEL,
  description: 'Inherits the parent harness, model, Host, and commands.',
}

export interface ChildSupervisorOptions<State> {
  deps: NodeDeps<State>
  tree: TreeContext<State>
  /** The owner node's id; its children are the store's rows under it. */
  ownerId: string
  cwd: string
  /** The owner's harness commands, before the `demi agent` injection: what an inheriting child receives. */
  parentCommands: Command[]
  /** The owner's prompt: what an inheriting child speaks with. */
  prompt: Pick<AgentHarness<State>, 'systemPrompt' | 'preamble'>
  /** When false, the owner may not spawn: its `demi agent` tree carries communication and reads only. */
  canSpawn: boolean
  /** When false, a child closing never wakes the idle owner; the product orchestrates the wakeup from the `closed` frame. */
  notifyParentOnIdle: boolean
  /** Invoked whenever the live-children set changes; wired to the owning job's settle loop. */
  onJobsChanged: (() => void) | null
  /** The node assembly: the supervisor asks it for every child and never builds one. */
  assemble(params: NodeParams<State>): Promise<AssembledNode<State>>
}

/**
 * The relationship module of one node (`docs/subagent.md`): its direct
 * children's lifecycle — spawn, resume, abort, the natural close and its
 * delivery — over the tree store, their frames, and the `demi agent`
 * command tree; communication and reads (`send` / `steer` / `show` /
 * `list`) resolve through the shared AgentDirectory and reach any live
 * agent in the tree. Every node — root or subagent — owns one.
 */
export class ChildSupervisor<State = unknown> {
  private readonly options: ChildSupervisorOptions<State>
  private readonly jobs = new Map<string, ChildJob<State>>()
  private parentSession: AgentSession<State> | null = null
  private isDisposed = false

  constructor(options: ChildSupervisorOptions<State>) {
    if (options.tree.profiles?.some((profile) => profile.name === INHERIT_PROFILE_NAME)) {
      throw new Error(`subagent profile name "${INHERIT_PROFILE_NAME}" is reserved: omitting --profile already inherits the parent`)
    }
    this.options = options
  }

  attachParent(session: AgentSession<State>): void {
    this.parentSession = session
  }

  ownerId(): string {
    return this.options.ownerId
  }

  hasLiveJobs(): boolean {
    return this.jobs.size > 0
  }

  /** The `agent` node shared by every session, with lifecycle authority scoped to its own children. */
  rootCommandNode(): CommandGroup {
    return subagentCommandNode<ChildJob<State>>({
      canSpawn: this.options.canSpawn,
      profileNames: () => this.configuredProfileNames(),
      spawn: (input) => this.spawn(input),
      resumeArchived: (id, message) => this.resumeArchived(id, message),
      attend: (job, ctx) => this.attendChild(job, ctx),
      getRunning: (id) => this.jobs.get(id) ?? null,
      send: (id, message) => this.deliverSend(id, message),
      steer: (id, message) => this.deliverSteer(id, message),
      abortSubtree: (id) => this.abortSubtree(id),
      tree: () => this.options.tree.directory.tree(),
      ownerId: () => this.ownerId(),
      show: (id) => {
        const entry = this.options.tree.directory.liveEntry(id)
        return entry ? { snapshot: entry.owner.snapshot(entry.job, true), text: entry.owner.renderShow(entry.job) } : null
      },
    })
  }

  hasShell(shellId: string): boolean {
    return this.environmentScopeForShell(shellId) !== null
  }

  /** Resolves the descendant scope owning a shell (recursively), for the command bridge dispatch. */
  environmentScopeForShell(
    shellId: string,
  ): { environment: ShellEnvironment; commandNames: ReadonlySet<string>; agentSessionId: string } | null {
    for (const job of this.jobs.values()) {
      for (const environment of job.node.environments()) {
        if (environment.getShell(shellId)) {
          return { environment, commandNames: job.node.commandNames, agentSessionId: job.id }
        }
      }
      const nested = job.node.supervisor.environmentScopeForShell(shellId)
      if (nested) return nested
    }
    return null
  }

  /** Re-emits `subagent started` + transcript reset for the whole live subtree (transcript resync). */
  replay(): void {
    for (const job of this.jobs.values()) {
      this.options.tree.emit({ type: 'subagent', event: 'started', job: this.wireJob(job) })
      const transcript = job.node.session.transcript()
      this.options.tree.emit({
        type: 'subagent_transcript_reset',
        subagentId: job.id,
        blocks: structuredClone(transcript.blocks),
        revision: transcript.revision,
      })
      job.node.supervisor.replay()
    }
  }

  /**
   * Detaches the live subtree on connection teardown: aborts in-flight turns,
   * flushes checkpoints, and keeps the persisted nodes so the next open of
   * the owner restores them — the same dispose semantics as the owner
   * session. No `closed` frame is emitted: the children are not done, just
   * paused.
   */
  async dispose(): Promise<void> {
    this.isDisposed = true
    for (const job of [...this.jobs.values()]) {
      job.isClosing = true
      job.unsubscribe()
      this.jobs.delete(job.id)
      this.options.tree.directory.unregister(job.id)
      await job.node.supervisor.dispose()
      await job.node.session.dispose().catch(noop)
      await job.node.disposeEnvironments()
      job.settleClosed({ phase: 'aborted' })
      job.wake?.()
    }
  }

  /**
   * Brings the owner's children back from the store (`docs/subagent.md` §
   * Persistence): a live child is rebuilt and continues what it was doing —
   * its interrupted turn, its queued messages — and closes if it is
   * quiescent; a closed child whose completion never reached the owner is
   * delivered now. Recursive: each restored child restores its own subtree.
   */
  async restore(): Promise<void> {
    if (!this.parentSession || this.isDisposed) return
    for (const record of await this.options.tree.store.children(this.options.ownerId)) {
      if (this.jobs.has(record.id)) continue
      if (record.closedPhase !== null) {
        if (!record.delivered) await this.deliverCompletion(record)
        continue
      }
      try {
        await this.restoreJob(record)
      } catch {
        // A child that cannot be rebuilt (its profile gone, its rows incomplete) drops with its subtree.
        await this.options.tree.store.deleteNode(record.id).catch(noop)
      }
    }
  }

  private async restoreJob(record: AgentNodeRecord): Promise<void> {
    const profile = this.resolveProfile(record.profileName ?? undefined)
    await this.startChild(record, profile, null)
  }

  /**
   * Revives an archived child: its node is live again in one commit — this
   * round's metadata, a fresh spawn time, the message queued — and the
   * session rebuilds from the preserved transcript with the message opening
   * its next turn on top of it.
   */
  private async resumeArchived(id: string, message: string): Promise<ChildJob<State>> {
    const parent = this.requireParent()
    if (this.isDisposed) throw new Error('owner session is closing')
    if (this.jobs.has(id)) throw new Error(`subagent "${id}" is still running; send or steer it instead`)
    if (this.jobs.size >= this.options.deps.maxLiveSubagents) {
      throw new Error(`at most ${this.options.deps.maxLiveSubagents} running subagents per session; abort one or wait for a result`)
    }
    const record = await this.options.tree.store.node(id)
    if (!record || record.closedPhase === null || record.parentId !== this.options.ownerId) {
      throw new Error(`no archived subagent "${id}" (see \`demi agent list\`)`)
    }
    // Validate before mutating: a profile that no longer exists must leave the archive intact.
    const profile = this.resolveProfile(record.profileName ?? undefined)
    const fields = { metadata: parent.actionMetadata(), spawnedAt: Date.now() }
    const content: UserContentBlock[] = [{ type: 'text', text: message }]
    await this.options.tree.store.reopenNode(id, fields, { id: createId(), text: textContentSummary(content), content })
    const live: AgentNodeRecord = { ...record, ...fields, closedPhase: null, closedAt: null, result: null, failure: null, delivered: false }
    return this.startChild(live, profile, null)
  }

  /** Every archived (finished, revivable) child of this owner, newest first. */
  async listArchivedJobs(): Promise<AgentNodeRecord[]> {
    if (!this.parentSession) return []
    const children = await this.options.tree.store.children(this.options.ownerId)
    return children
      .filter((record) => record.closedPhase !== null && !this.jobs.has(record.id))
      .sort((a, b) => (b.closedAt ?? 0) - (a.closedAt ?? 0))
  }

  async abortSubtree(id: string): Promise<void> {
    const job = this.jobs.get(id)
    if (!job) return
    await this.closeJob(job, 'aborted')
  }

  /** Aborts every live child (each with its subtree); the archive is untouched. */
  async abortAll(): Promise<void> {
    for (const id of [...this.jobs.keys()]) await this.abortSubtree(id)
  }

  private async spawn(input: {
    prompt: string
    profileName: string | undefined
    description: string
    isSpawnForbidden: boolean
  }): Promise<ChildJob<State>> {
    const parent = this.requireParent()
    if (!this.options.canSpawn) throw new Error('this session may not spawn subagents')
    if (this.isDisposed) throw new Error('owner session is closing')
    if (this.jobs.size >= this.options.deps.maxLiveSubagents) {
      throw new Error(`at most ${this.options.deps.maxLiveSubagents} running subagents per session; abort one or wait for a result`)
    }
    const profile = this.resolveProfile(input.profileName)
    const record: AgentNodeRecord = {
      id: createId(),
      parentId: this.options.ownerId,
      description: input.description,
      profileName: input.profileName ?? null,
      metadata: parent.actionMetadata(),
      spawnedAt: Date.now(),
      canSpawnSubagents: !input.isSpawnForbidden && profile.canSpawnSubagents !== false,
      closedPhase: null,
      closedAt: null,
      result: null,
      failure: null,
      delivered: false,
    }
    const content: UserContentBlock[] = [{ type: 'text', text: input.prompt }]
    return this.startChild(record, profile, { id: createId(), text: textContentSummary(content), content })
  }

  /**
   * Everything spawn, resume and restore share: the child's node from the
   * assembly — fresh with its brief queued in the create commit, or from the
   * store — then the job, the continuation of what it has to run, its own
   * subtree, and the settle loop that closes it when it is done.
   */
  private async startChild(record: AgentNodeRecord, profile: SubagentProfile<State>, firstMessage: QueuedMessage | null): Promise<ChildJob<State>> {
    const release = await this.options.deps.activity(this.options.tree.hostSessionId).enter()
    try {
      const parent = this.requireParent()
      const inherited = profile.commands ? profile.commands([...this.options.parentCommands]) : [...this.options.parentCommands]
      let job: ChildJob<State> | null = null
      const { node } = await this.options.assemble({
        record,
        cwd: this.options.cwd,
        provider: parent.cloneProviderRuntime(),
        model: profile.model ?? structuredClone(parent.modelSelection),
        prompt: {
          systemPrompt: profile.systemPrompt?.bind(this.options.deps.agent) ?? this.options.prompt.systemPrompt,
          preamble: profile.systemPrompt ? undefined : this.options.prompt.preamble,
        },
        preambleSuffix: this.subagentPreamble(record.id, record.canSpawnSubagents),
        commands: () => inherited,
        shellEnv: { DEMI_SUBAGENT_ID: record.id, DEMI_PARENT_SESSION_ID: this.options.ownerId },
        policy: CHILD_POLICY,
        firstMessage,
        onJobsChanged: () => job?.wake?.(),
      })
      job = this.attachNode(node, record)
      const tracked = job
      node.continue((turn) => this.trackTurn(tracked, turn))
      await node.supervisor.restore()
      void this.settleJob(job)
      return job
    } finally { release() }
  }

  private attachNode(node: SessionNode<State>, record: AgentNodeRecord): ChildJob<State> {
    let settleClosed!: (close: SubagentClose) => void
    const closed = new Promise<SubagentClose>((resolve) => {
      settleClosed = resolve
    })
    const job: ChildJob<State> = {
      id: record.id,
      node,
      description: record.description,
      profileName: record.profileName,
      metadata: record.metadata,
      spawnedAt: record.spawnedAt,
      lastEventAt: Date.now(),
      tools: [],
      lastAssistantTextAt: null,
      phase: 'running',
      failure: null,
      unsubscribe: noop,
      closed,
      settleClosed,
      isClosing: false,
      attended: false,
      wake: null,
    }
    job.unsubscribe = node.session.subscribe((event) => {
      if (event.type === 'phase_changed') {
        job.wake?.()
        return
      }
      if (event.type !== 'transcript_changed') return
      this.recordTelemetry(job, event.patches)
      this.options.tree.emit({
        type: 'subagent_transcript_patch',
        subagentId: job.id,
        patches: event.patches,
        revision: event.revision,
      })
    })
    this.jobs.set(job.id, job)
    this.options.tree.directory.register(job, this)
    this.options.onJobsChanged?.()
    this.options.tree.emit({ type: 'subagent', event: 'started', job: this.wireJob(job) })
    const transcript = node.session.transcript()
    this.options.tree.emit({
      type: 'subagent_transcript_reset',
      subagentId: job.id,
      blocks: structuredClone(transcript.blocks),
      revision: transcript.revision,
    })
    return job
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

    job.attended = true
    const close = await job.closed
    job.attended = false
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

  /**
   * Resolves a send/steer target against the directory. `parent` is the
   * session that spawned the caller; a caller cannot message itself.
   */
  private resolveTarget(rawId: string): {
    id: string
    session: AgentSession<State>
    job: ChildJob<State> | null
    owner: ChildSupervisor<State> | null
  } {
    const directory = this.options.tree.directory
    const selfId = this.ownerId()
    let id = rawId
    if (id === 'parent') {
      const parentId = directory.parentIdOf(selfId)
      if (parentId === null) throw new Error('the root session has no parent')
      if (parentId === undefined) throw new Error('this session is not in the agent directory')
      id = parentId
    }
    if (id === selfId) throw new Error('cannot message your own session')
    if (id === directory.rootId()) {
      return { id, session: directory.rootSession(), job: null, owner: null }
    }
    const entry = directory.liveEntry(id)
    if (!entry || entry.job.isClosing) {
      throw new Error(`no live agent "${id}" (see \`demi agent list\`; an archived child is revived only by its parent via resume)`)
    }
    return { id, session: entry.job.node.session, job: entry.job, owner: entry.owner }
  }

  /**
   * Mailbox delivery: an ordinary user send on the target session. The
   * session's own action queue is the inbox — a busy target sees it as a new
   * user turn after the current one; an idle root wakes; a finishing subagent
   * is kept open for one more turn by its settle loop (the enqueue lands
   * before the loop's synchronous close check, so nothing drops silently).
   */
  private deliverSend(rawId: string, message: string): string {
    const target = this.resolveTarget(rawId)
    const content: UserContentBlock[] = [{ type: 'text', text: `${this.senderPrefix()} ${message}` }]
    const metadata = target.job ? target.job.metadata : this.senderMetadata()
    if (target.job && target.owner) {
      target.owner.trackTurn(target.job, target.session.send(content, metadata ? { metadata } : {}))
      target.job.wake?.()
    } else {
      void target.session.send(content, metadata ? { metadata } : {}).catch(noop)
    }
    return target.id
  }

  /** Chime-in delivery: injects into the target's running turn; no fallback when idle. */
  private async deliverSteer(rawId: string, message: string): Promise<string> {
    const target = this.resolveTarget(rawId)
    if (target.session.phase() === 'idle') {
      throw new Error(`agent "${target.id}" has no running turn to steer; use \`demi agent send\``)
    }
    const content: UserContentBlock[] = [{ type: 'text', text: `${this.senderPrefix()} ${message}` }]
    await target.session.steer(content)
    return target.id
  }

  private senderPrefix(): string {
    const selfId = this.ownerId()
    const entry = this.options.tree.directory.liveEntry(selfId)
    const description = entry ? entry.job.description : 'root session'
    return `[agent ${selfId}${description ? ` — ${description}` : ''}]`
  }

  /** Metadata for a turn on the root: the sender subtree's spawning round, or null from the root itself. */
  private senderMetadata(): AgentMetadata | null {
    return this.options.tree.directory.liveEntry(this.ownerId())?.job.metadata ?? null
  }

  /** Delivers one stdin chunk from the attending spawner: mid-turn as a steer, otherwise as a new user turn. */
  private async steerChild(job: ChildJob<State>, message: string): Promise<void> {
    const content: UserContentBlock[] = [{ type: 'text', text: message }]
    if (job.node.session.phase() !== 'idle') {
      try {
        await job.node.session.steer(content)
        return
      } catch {
        // Turn boundary raced the steer; fall through to a fresh turn.
      }
    }
    this.trackTurn(job, job.node.session.send(content))
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

  /** Observes one turn promise of an own child: a non-abort failure closes the job as an error. */
  private trackTurn(job: ChildJob<State>, turn: Promise<void>): void {
    turn.catch((error: unknown) => {
      if (job.isClosing) return
      job.failure = errorMessage(error)
      void this.closeJob(job, 'error')
    })
  }

  /**
   * The one place a child closes naturally — the close-when-done policy.
   * Loops until the child is quiescent: no running or queued turn (the
   * session action queue doubles as the mailbox), no pending yield wakeups,
   * and no live children of its own. The final check-and-close is
   * synchronous, so a send that lands before it is processed and one that
   * lands after it fails on `isClosing` — nothing drops silently.
   */
  private async settleJob(job: ChildJob<State>): Promise<void> {
    while (!job.isClosing) {
      if (job.node.session.isSettled() && !job.node.session.hasPendingYields() && !job.node.supervisor.hasLiveJobs()) {
        void this.closeJob(job, 'completed')
        return
      }
      await new Promise<void>((resolve) => {
        let settled = false
        const finish = (): void => {
          if (settled) return
          settled = true
          job.wake = null
          resolve()
        }
        job.wake = finish
        void job.closed.then(finish)
        if (job.node.session.isSettled()) return
        void job.node.session.waitUntilDone().then(finish)
      })
    }
  }

  private async closeJob(job: ChildJob<State>, phase: AgentNodeClosePhase): Promise<void> {
    if (job.isClosing) return
    job.isClosing = true
    job.phase = phase
    job.unsubscribe()
    this.jobs.delete(job.id)
    this.options.tree.directory.unregister(job.id)
    job.wake?.()

    // A natural completion has no live descendants by construction; an abort
    // or error tears the subtree down with it.
    await job.node.supervisor.abortAll()

    const result = phase === 'completed' ? boundedResultText(lastAssistantText(job.node.session.transcript().blocks)) : null
    // dispose() flushes the final checkpoint; the close row is the next commit
    // (`docs/subagent.md` § Persistence), the node quiescent between the two.
    await job.node.session.dispose().catch(noop)
    await job.node.disposeEnvironments()
    await this.options.tree.store.closeNode(job.id, { phase, closedAt: Date.now(), result, failure: job.failure }).catch(noop)

    const close: SubagentClose = {
      phase,
      ...(result !== null ? { result } : {}),
      ...(job.failure ? { failure: job.failure } : {}),
    }
    this.options.tree.emit({ type: 'subagent', event: 'closed', job: this.wireJob(job, result ?? undefined) })
    job.settleClosed(close)
    this.options.onJobsChanged?.()
    await this.deliverClose({ id: job.id, description: job.description, metadata: job.metadata, phase, result, failure: job.failure }, job.attended)
  }

  /**
   * Where a completion goes (`docs/subagent.md` § Persistence): a product that
   * drives the root itself has the `closed` frame, and an owner busy in the
   * spawn command has that command's exit — the store is told at once in both
   * cases. Otherwise the owner gets a user message — now if idle, at its next
   * turn boundary if busy — that its own checkpoint records as delivered. A
   * detached owner waits for the restore.
   */
  private async deliverClose(completion: Completion, attended: boolean): Promise<void> {
    const parent = this.parentSession
    if (!parent || this.isDisposed) return
    if (!this.options.notifyParentOnIdle || (attended && parent.phase() !== 'idle')) {
      await this.options.tree.store.markDelivered(completion.id).catch(noop)
      return
    }
    this.sendCompletion(parent, completion)
  }

  /** A completion the owner never received before the process ended, from the store at restore. */
  private async deliverCompletion(record: AgentNodeRecord): Promise<void> {
    const parent = this.requireParent()
    if (!this.options.notifyParentOnIdle || record.closedPhase === null) {
      await this.options.tree.store.markDelivered(record.id).catch(noop)
      return
    }
    this.sendCompletion(parent, {
      id: record.id,
      description: record.description,
      metadata: record.metadata,
      phase: record.closedPhase,
      result: record.result,
      failure: record.failure,
    })
  }

  private sendCompletion(parent: AgentSession<State>, completion: Completion): void {
    const label = `subagent ${completion.id}${completion.description ? ` — ${completion.description}` : ''}`
    const body =
      completion.phase === 'completed'
        ? `[${label}] completed.\nResult:\n${completion.result || '(empty)'}`
        : completion.phase === 'aborted'
          ? `[${label}] aborted.`
          : `[${label}] failed: ${completion.failure ?? 'unknown error'}`
    // The wakeup round runs on behalf of the round that spawned the child, so it
    // carries that round's metadata; its id names the child, so the owner's
    // checkpoint that carries it marks the completion delivered.
    void parent
      .send([{ type: 'text', text: body }], {
        id: completionMessageId(completion.id),
        ...(completion.metadata ? { metadata: completion.metadata } : {}),
      })
      .catch(noop)
  }

  private requireParent(): AgentSession<State> {
    if (!this.parentSession) throw new Error('subagent supervisor has no owner session')
    return this.parentSession
  }

  /**
   * No name means the unnamed inherit profile: the parent harness, model, Host
   * and commands, always available and never configurable. A name must match a
   * declared profile; "default" is not a name.
   */
  private resolveProfile(name: string | undefined): SubagentProfile<State> {
    if (name === undefined) return INHERIT_PROFILE as SubagentProfile<State>
    const profile = this.options.tree.profiles?.find((candidate) => candidate.name === name)
    if (profile) return profile
    const names = this.configuredProfileNames()
    throw new Error(`unknown profile "${name}" (available: ${names.length > 0 ? names.join(', ') : 'none; omit --profile to inherit the parent'})`)
  }

  private configuredProfileNames(): string[] {
    return (this.options.tree.profiles ?? []).map((profile) => profile.name)
  }

  private subagentPreamble(childId: string, canSpawn: boolean): string {
    return [
      `You are a subagent: an isolated child agent session (id ${childId}) spawned by parent agent session ${this.ownerId()}. Your transcript starts empty; the task brief in the first user message is your entire context.`,
      'When you end your turn with nothing pending — no queued messages, no scheduled wakeups, no running children of your own — the session ends and your last assistant text is returned to the parent as the result. Write it for the parent agent, in the shape the task brief asked for.',
      canSpawn ? '`demi agent spawn` spawns your own children.' : 'This session may not spawn subagents.',
      '`demi agent send <id|parent> <message>` leaves a message any live agent sees at its next turn boundary; `demi agent steer <id> <message>` chimes into a running agent\'s current turn; `demi agent list` renders the whole agent tree with your position.',
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
        const block = job.node.session.transcript().blocks[patch.path[1]]
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
    const phase = job.node.session.phase()
    if (phase === 'compacting') return 'compacting'
    if (phase === 'idle') return job.node.session.hasPendingYields() ? 'pending_yield' : 'idle'
    return job.node.session.turnPhase() ?? 'provider_streaming'
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

  snapshot(job: ChildJob<State>, detailed: boolean): Record<string, unknown> {
    const now = Date.now()
    const execution = this.executionOf(job)
    const base: Record<string, unknown> = {
      subagentId: job.id,
      parentSessionId: this.ownerId(),
      description: job.description,
      profile: job.profileName,
      phase: job.phase,
      elapsedMs: now - job.spawnedAt,
      lastEventMs: now - job.lastEventAt,
      execution,
      activity: this.activityOf(job, execution),
    }
    if (!detailed) return base
    const text = lastAssistantText(job.node.session.transcript().blocks)
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

  renderListLine(job: ChildJob<State>): string {
    const now = Date.now()
    const execution = this.executionOf(job)
    const parts = [
      job.id,
      job.phase,
      `up ${formatDuration(now - job.spawnedAt)}`,
      `last-event ${formatDuration(now - job.lastEventAt)} ago`,
      `profile=${job.profileName ?? INHERIT_PROFILE_LABEL}`,
      job.description ? `"${job.description}"` : '(no description)',
      `execution=${execution}`,
      `activity=${this.activityOf(job, execution)}`,
    ]
    return parts.join('  ')
  }

  renderShow(job: ChildJob<State>): string {
    const now = Date.now()
    const execution = this.executionOf(job)
    const lines = [
      `id: ${job.id}`,
      `parent: ${this.ownerId()}`,
      `description: ${job.description || '(none)'}`,
      `profile: ${job.profileName ?? INHERIT_PROFILE_LABEL}`,
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
    const text = boundedResultText(lastAssistantText(job.node.session.transcript().blocks))
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
      parentSessionId: this.ownerId(),
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
