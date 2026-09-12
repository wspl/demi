import { z } from 'zod'
import {
  ActivityGate,
  SerialQueue,
  createId,
  errorMessage,
  noop,
  utf8Slice,
} from '@demicodes/utils'
import type {
  Command,
  CommandGroup,
  CommandStorage,
  ShellEnvironment,
} from '@demicodes/shell'
import type { Block, QueuedMessage, UserContentBlock } from '@demicodes/core'
import { textContentSummary, type AgentSession } from '../session/session'
import type { ServerFrame, SubagentJob, TranscriptPatch } from '../protocol/frames'
import type {
  AgentHarness,
  AgentMetadata,
  AgentNodeClosePhase,
  AgentNodeRecord,
  ExternalMutationReservation,
  SubagentProfile,
} from '../types'
import type {
  AssembledNode,
  NodeDeps,
  NodeParams,
  TreeContext,
} from '../node/assemble'
import { CHILD_POLICY, type SessionNode } from '../node/node'
import { completionMessageId } from '../store/tree-store'
import {
  injectSubagentCommand,
  subagentCommandNode,
  subagentCommandShape,
} from './commands'
import { formatDuration } from './format'

export { injectSubagentCommand, subagentCommandShape }

/**
 * Default per-session live-children ceiling; override with
 * `AgentServerOptions.subagents.maxLiveSubagents`.
 */
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

const startInputSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('spawn'),
    prompt: z.string(),
    profileName: z.string().nullable(),
    description: z.string(),
    isSpawnForbidden: z.boolean(),
  }).strict(),
  z.object({
    kind: z.literal('resume'),
    id: z.string(),
    message: z.string(),
  }).strict(),
])
const startReceiptSchema = z.object({
  input: startInputSchema,
  nodeId: z.string(),
  spawnedAt: z.number().int().nonnegative(),
}).strict()
type StartInput = z.infer<typeof startInputSchema>

interface SubagentClose {
  phase: AgentNodeClosePhase
  result?: string
  failure?: string
}

/** A closed child as its parent hears of it, live or from the store. */
interface Completion {
  id: string
  spawnedAt: number
  closedAt: number
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

/**
 * A live child as its supervisor sees it: the node plus the relationship's
 * bookkeeping.
 */
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
  /**
   * Wakes the settle loop when the child set or an inbound send changes the
   * picture.
   */
  wake: (() => void) | null
}

/**
 * The reserved word a harness may not use as a profile name: it is not a
 * profile, it is the absence of one.
 */
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
  ownerRound: number
  cwd: string
  /**
   * The owner's harness commands, before the `demi agent` injection: what an
   * inheriting child receives.
   */
  parentCommands: Command[]
  /** The owner's prompt: what an inheriting child speaks with. */
  prompt: Pick<AgentHarness<State>, 'systemPrompt' | 'preamble'>
  /**
   * When false, the owner may not spawn: its `demi agent` tree carries
   * communication and reads only.
   */
  canSpawn: boolean
  /**
   * When false, a child closing never wakes the idle owner; the product
   * orchestrates the wakeup from the `closed` frame.
   */
  notifyParentOnIdle: boolean
  /**
   * Invoked whenever the live-children set changes; wired to the owning job's
   * settle loop.
   */
  onJobsChanged: (() => void) | null
  /**
   * The node assembly: the supervisor asks it for every child and never builds
   * one.
   */
  assemble(params: NodeParams<State>): Promise<AssembledNode<State>>
}

/**
 * The relationship module of one node (`docs/subagent.md`): its direct
 * children's lifecycle — spawn, resume, abort, the natural close and its
 * delivery — over the tree store, their frames, and the `demi agent`
 * command tree; communication and reads (`send` / `show` /
 * `list`) resolve through the shared AgentDirectory and reach any live
 * agent in the tree. Every node — root or subagent — owns one.
 */
export class ChildSupervisor<State = unknown> {
  private readonly starts = new SerialQueue()
  private readonly options: ChildSupervisorOptions<State>
  private readonly jobs = new Map<string, ChildJob<State>>()
  private parentSession: AgentSession<State> | null = null
  private isDisposed = false
  private readonly lifecycle = new ActivityGate()

  constructor(options: ChildSupervisorOptions<State>) {
    if (
      options.tree.profiles?.some((profile) => profile.name === INHERIT_PROFILE_NAME)
    ) {
      throw new Error(
        `subagent profile name "${INHERIT_PROFILE_NAME}" is reserved: omitting --profile already inherits the parent`,
      )
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

  /** Refuses edits while a child can still deliver work to the parent. */
  async reserveEdit(): Promise<ExternalMutationReservation> {
    const release = this.lifecycle.tryReserve()
    if (!release) {
      throw new Error('Cannot edit while a child lifecycle operation is in progress')
    }
    try {
      const children = await this.options.tree.store.children(this.options.ownerId)
      if (this.isDisposed || this.jobs.size > 0
        || children.some((child) => child.closedPhase === null || !child.delivered)) {
        throw new Error('Cannot edit while children or completion notifications are pending')
      }
      return { release }
    } catch (error) {
      release()
      throw error
    }
  }

  private async withLifecycleAccess<T>(operation: () => Promise<T>): Promise<T> {
    const release = this.lifecycle.tryEnter()
    if (!release) {
      throw new Error('Cannot change children while a transcript edit is being prepared')
    }
    try {
      return await operation()
    } finally {
      release()
    }
  }

  /** Reserve the operation identity before creating a node, so retry can finish or reuse it. */
  private startRequest(
    input: StartInput,
    requestId: string,
    storage: CommandStorage,
  ): Promise<string> {
    return this.starts.run(() => this.withLifecycleAccess(async () => {
      if (this.isDisposed) {
        throw new Error('owner session is closing')
      }
      const key = `agent.start.${requestId}`
      const current = await storage.readJson<unknown>(key)
      let receipt: z.infer<typeof startReceiptSchema>
      if (current !== null) {
        receipt = startReceiptSchema.parse(current)
        if (JSON.stringify(receipt.input) !== JSON.stringify(startInputSchema.parse(input))) {
          throw new Error('request-id already belongs to different agent arguments')
        }
      } else {
        const previous = input.kind === 'resume'
          ? await this.options.tree.store.node(input.id)
          : null
        receipt = {
          input,
          nodeId: input.kind === 'resume' ? input.id : createId(),
          spawnedAt: Math.max(Date.now(), (previous?.spawnedAt ?? 0) + 1),
        }
        await storage.writeJson(key, receipt)
      }
      const record = await this.options.tree.store.node(receipt.nodeId)
      if (record) {
        if (record.parentId !== this.options.ownerId) {
          throw new Error('request-id references an agent owned by another session')
        }
        if (input.kind === 'spawn' || record.spawnedAt === receipt.spawnedAt) {
          if (record.closedPhase === null && !this.jobs.has(record.id)) {
            await this.restoreJob(record)
          }
          return record.id
        }
        if (record.spawnedAt > receipt.spawnedAt) {
          throw new Error('resume request has been superseded by a later round')
        }
      }
      const job = input.kind === 'spawn'
        ? await this.spawn({ ...input, profileName: input.profileName ?? undefined }, receipt.nodeId, receipt.spawnedAt)
        : await this.resumeArchived(input.id, input.message, receipt.spawnedAt)
      return job.id
    }))
  }

  /**
   * The `agent` node shared by every session, with lifecycle authority scoped
   * to its own children.
   */
  rootCommandNode(): CommandGroup {
    return subagentCommandNode({
      canSpawn: this.options.canSpawn,
      profileNames: () => this.configuredProfileNames(),
      spawn: (input, requestId, storage) => this.startRequest({
        ...input,
        kind: 'spawn',
        profileName: input.profileName ?? null,
      }, requestId, storage),
      resumeArchived: (id, message, requestId, storage) =>
        this.startRequest({ kind: 'resume', id, message }, requestId, storage),
      getRunning: (id) => this.jobs.get(id) ?? null,
      send: (id, message) => this.deliverSend(id, message),
      abortSubtree: (id) => this.abortSubtree(id),
      tree: () => this.options.tree.directory.tree(),
      ownerId: () => this.ownerId(),
      show: (id) => {
        const entry = this.options.tree.directory.liveEntry(id)
        return entry
          ? {
              snapshot: entry.owner.snapshot(entry.job, true),
              text: entry.owner.renderShow(entry.job),
            }
          : null
      },
    })
  }

  hasShell(shellId: string): boolean {
    return this.environmentScopeForShell(shellId) !== null
  }

  /**
   * Resolves the descendant scope owning a shell (recursively), for the command
   * bridge dispatch.
   */
  environmentScopeForShell(shellId: string): {
    environment: ShellEnvironment
    commandNames: ReadonlySet<string>
    agentSessionId: string
  } | null {
    for (const job of this.jobs.values()) {
      for (const environment of job.node.environments()) {
        if (environment.getShell(shellId)) {
          return {
            environment,
            commandNames: job.node.commandNames,
            agentSessionId: job.id,
          }
        }
      }
      const nested = job.node.supervisor.environmentScopeForShell(shellId)
      if (nested) {
        return nested
      }
    }
    return null
  }

  /**
   * Re-emits `subagent started` + transcript reset for the whole live subtree
   * (transcript resync).
   */
  replay(): void {
    for (const job of this.jobs.values()) {
      this.options.tree.emit({
        type: 'subagent',
        event: 'started',
        job: this.wireJob(job),
      })
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
    await this.starts.settled()
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
    await this.withLifecycleAccess(() => this.restoreChildren())
  }

  private async restoreChildren(): Promise<void> {
    if (!this.parentSession || this.isDisposed) {
      return
    }
    for (const record of await this.options.tree.store.children(
      this.options.ownerId,
    )) {
      if (this.jobs.has(record.id)) {
        continue
      }
      if (record.closedPhase !== null) {
        if (!record.delivered) {
          await this.deliverCompletion(record)
        }
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
  private async resumeArchived(
    id: string,
    message: string,
    spawnedAt: number,
  ): Promise<ChildJob<State>> {
    const parent = this.requireParent()
    if (this.isDisposed) {
      throw new Error('owner session is closing')
    }
    if (this.jobs.has(id)) {
      throw new Error(`subagent "${id}" is still running; send it a message instead`)
    }
    if (this.jobs.size >= this.options.deps.maxLiveSubagents) {
      throw new Error(
        `at most ${this.options.deps.maxLiveSubagents} running subagents per session; abort one or wait for a result`,
      )
    }
    const record = await this.options.tree.store.node(id)
    if (
      !record ||
      record.closedPhase === null ||
      record.parentId !== this.options.ownerId
    ) {
      throw new Error(`no archived subagent "${id}" (see \`demi agent list\`)`)
    }
    if (!record.delivered) {
      throw new Error('The previous completion is not saved by the parent yet; retry resume after receiving it')
    }
    // Validate before mutating: a profile that no longer exists must leave the archive intact.
    const profile = this.resolveProfile(record.profileName ?? undefined)
    const fields = {
      metadata: parent.actionMetadata(),
      spawnedAt,
    }
    const content: UserContentBlock[] = [
      {
        type: 'text',
        text: message,
      },
    ]
    await this.options.tree.store.reopenNode(id, fields, {
      id: createId(),
      text: textContentSummary(content),
      content,
    })
    const live: AgentNodeRecord = {
      ...record,
      ...fields,
      closedPhase: null,
      closedAt: null,
      result: null,
      failure: null,
      delivered: false,
    }
    return this.startChild(live, profile, null)
  }

  /** Every archived (finished, revivable) child of this owner, newest first. */
  async listArchivedJobs(): Promise<AgentNodeRecord[]> {
    if (!this.parentSession) {
      return []
    }
    const children = await this.options.tree.store.children(this.options.ownerId)
    return children
      .filter((record) => record.closedPhase !== null && !this.jobs.has(record.id))
      .sort((a, b) => (b.closedAt ?? 0) - (a.closedAt ?? 0))
  }

  async abortSubtree(id: string): Promise<void> {
    const job = this.jobs.get(id)
    if (!job) {
      return
    }
    await this.closeJob(job, 'aborted')
  }

  /**
   * Aborts every live child (each with its subtree); the archive is untouched.
   */
  async abortAll(): Promise<void> {
    for (const id of [...this.jobs.keys()]) {
      await this.abortSubtree(id)
    }
  }

  private async spawn(input: {
    prompt: string
    profileName: string | undefined
    description: string
    isSpawnForbidden: boolean
  }, id: string, spawnedAt: number): Promise<ChildJob<State>> {
    const parent = this.requireParent()
    if (!this.options.canSpawn) {
      throw new Error('this session may not spawn subagents')
    }
    if (this.isDisposed) {
      throw new Error('owner session is closing')
    }
    if (this.jobs.size >= this.options.deps.maxLiveSubagents) {
      throw new Error(
        `at most ${this.options.deps.maxLiveSubagents} running subagents per session; abort one or wait for a result`,
      )
    }
    const profile = this.resolveProfile(input.profileName)
    const record: AgentNodeRecord = {
      id,
      parentId: this.options.ownerId,
      description: input.description,
      profileName: input.profileName ?? null,
      metadata: parent.actionMetadata(),
      spawnedAt,
      canSpawnSubagents:
        !input.isSpawnForbidden && profile.canSpawnSubagents !== false,
      closedPhase: null,
      closedAt: null,
      result: null,
      failure: null,
      delivered: false,
    }
    const content: UserContentBlock[] = [
      {
        type: 'text',
        text: input.prompt,
      },
    ]
    return this.startChild(record, profile, {
      id: createId(),
      text: textContentSummary(content),
      content,
    })
  }

  /**
   * Everything spawn, resume and restore share: the child's node from the
   * assembly — fresh with its brief queued in the create commit, or from the
   * store — then the job, the continuation of what it has to run, its own
   * subtree, and the settle loop that closes it when it is done.
   */
  private async startChild(
    record: AgentNodeRecord,
    profile: SubagentProfile<State>,
    firstMessage: QueuedMessage | null,
  ): Promise<ChildJob<State>> {
    const release = await this.options.deps
      .activity(this.options.tree.hostSessionId)
      .enter()
    try {
      const parent = this.requireParent()
      const inherited = profile.commands
        ? profile.commands([...this.options.parentCommands])
        : [...this.options.parentCommands]
      let job: ChildJob<State> | null = null
      const { node } = await this.options.assemble({
        record,
        cwd: this.options.cwd,
        provider: parent.cloneProviderRuntime(),
        model: profile.model ?? structuredClone(parent.modelSelection),
        prompt: {
          systemPrompt:
            profile.systemPrompt?.bind(this.options.deps.agent) ??
            this.options.prompt.systemPrompt,
          preamble: profile.systemPrompt ? undefined : this.options.prompt.preamble,
        },
        preambleSuffix: this.subagentPreamble(record.id, record.canSpawnSubagents),
        commands: () => inherited,
        shellEnv: {
          DEMI_SUBAGENT_ID: record.id,
          DEMI_PARENT_SESSION_ID: this.options.ownerId,
        },
        policy: CHILD_POLICY,
        firstMessage,
        onJobsChanged: () => job?.wake?.(),
      })
      job = this.attachNode(node, record)
      const tracked = job
      node.continue((turn) => this.trackTurn(tracked, turn))
      await node.supervisor.restore()
      void this.settleJob(job).catch(error => this.reportLifecycleFailure(error))
      return job
    } finally {
      release()
    }
  }

  private attachNode(
    node: SessionNode<State>,
    record: AgentNodeRecord,
  ): ChildJob<State> {
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
      wake: null,
    }
    job.unsubscribe = node.session.subscribe((event) => {
      if (event.type === 'action_failed') {
        job.failure = errorMessage(event.error)
        void this.closeJob(job, 'error').catch(error => this.reportLifecycleFailure(error))
        return
      }
      if (event.type === 'phase_changed') {
        job.wake?.()
        return
      }
      if (event.type !== 'transcript_changed') {
        return
      }
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
    this.options.tree.emit({
      type: 'subagent',
      event: 'started',
      job: this.wireJob(job),
    })
    const transcript = node.session.transcript()
    this.options.tree.emit({
      type: 'subagent_transcript_reset',
      subagentId: job.id,
      blocks: structuredClone(transcript.blocks),
      revision: transcript.revision,
    })
    return job
  }

  /**
   * Resolves a message target against the directory. `parent` is the
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
      if (parentId === null) {
        throw new Error('the root session has no parent')
      }
      if (parentId === undefined) {
        throw new Error('this session is not in the agent directory')
      }
      id = parentId
    }
    if (id === selfId) {
      throw new Error('cannot message your own session')
    }
    if (id === directory.rootId()) {
      return {
        id,
        session: directory.rootSession(),
        job: null,
        owner: null,
      }
    }
    const entry = directory.liveEntry(id)
    if (!entry || entry.job.isClosing) {
      throw new Error(
        `no live agent "${id}" (see \`demi agent list\`; an archived child is revived only by its parent via resume)`,
      )
    }
    return {
      id,
      session: entry.job.node.session,
      job: entry.job,
      owner: entry.owner,
    }
  }

  private async deliverSend(rawId: string, content: string): Promise<string> {
    const target = this.resolveTarget(rawId)
    const senderId = this.ownerId()
    const entry = this.options.tree.directory.liveEntry(senderId)
    await target.session.acceptAgentMessage({
      id: createId(),
      sender: {
        id: senderId,
        description: entry?.job.description ?? 'root session',
        round: this.options.ownerRound,
      },
      recipientId: target.id,
      timestamp: new Date().toISOString(),
      content,
      event: { type: 'message' },
    }, target.job?.metadata ?? this.senderMetadata())
    target.job?.wake?.()
    return target.id
  }

  /**
   * Metadata for a turn on the root: the sender subtree's spawning round, or
   * null from the root itself.
   */
  private senderMetadata(): AgentMetadata | null {
    return (
      this.options.tree.directory.liveEntry(this.ownerId())?.job.metadata ?? null
    )
  }

  /**
   * Observes one turn promise of an own child: a non-abort failure closes the
   * job as an error.
   */
  private trackTurn(job: ChildJob<State>, turn: Promise<void>): void {
    turn.catch((error: unknown) => {
      if (job.isClosing) {
        return
      }
      job.failure = errorMessage(error)
      void this.closeJob(job, 'error').catch(error => this.reportLifecycleFailure(error))
    })
  }

  /**
   * The one place a child closes naturally — the close-when-done policy.
   * Loops until the child is quiescent: no running or queued action, no unread internal input, no pending yield wakeups,
   * and no live children of its own. The final check-and-close is
   * synchronous, so a send that lands before it is processed and one that
   * lands after it fails on `isClosing` — nothing drops silently.
   */
  private async settleJob(job: ChildJob<State>): Promise<void> {
    while (!job.isClosing) {
      if (
        job.node.session.isSettled() &&
        !job.node.session.hasPendingAgentMessages() &&
        !job.node.session.hasPendingYields() &&
        !job.node.supervisor.hasLiveJobs()
      ) {
        await this.closeJob(job, 'completed')
        return
      }
      await new Promise<void>((resolve) => {
        let settled = false
        const finish = (): void => {
          if (settled) {
            return
          }
          settled = true
          job.wake = null
          resolve()
        }
        job.wake = finish
        void job.closed.then(finish)
        if (job.node.session.isSettled()) {
          return
        }
        void job.node.session.waitUntilDone().then(finish)
      })
    }
  }

  private async closeJob(
    job: ChildJob<State>,
    phase: AgentNodeClosePhase,
  ): Promise<void> {
    await this.withLifecycleAccess(() => this.closeAdmittedJob(job, phase))
  }

  private async closeAdmittedJob(
    job: ChildJob<State>,
    phase: AgentNodeClosePhase,
  ): Promise<void> {
    if (job.isClosing) {
      return
    }
    job.isClosing = true
    job.phase = phase
    job.unsubscribe()
    job.wake?.()

    // A natural completion has no live descendants by construction; an abort
    // or error tears the subtree down with it.
    await job.node.supervisor.abortAll()

    const result =
      phase === 'completed'
        ? boundedResultText(lastAssistantText(job.node.session.transcript().blocks))
        : null
    // dispose() flushes the final checkpoint; the close row is the next commit
    // (`docs/subagent.md` § Persistence), the node quiescent between the two.
    await job.node.session.dispose()
    await job.node.disposeEnvironments()
    const closedAt = Date.now()
    await this.options.tree.store
      .closeNode(job.id, {
        phase,
        closedAt,
        result,
        failure: job.failure,
      })
    this.jobs.delete(job.id)
    this.options.tree.directory.unregister(job.id)

    const close: SubagentClose = {
      phase,
      ...(result !== null ? { result } : {}),
      ...(job.failure ? { failure: job.failure } : {}),
    }
    this.options.tree.emit({
      type: 'subagent',
      event: 'closed',
      job: this.wireJob(job, result ?? undefined, closedAt),
    })
    job.settleClosed(close)
    this.options.onJobsChanged?.()
    await this.deliverClose(
      {
        id: job.id,
        spawnedAt: job.spawnedAt,
        closedAt,
        description: job.description,
        metadata: job.metadata,
        phase,
        result,
        failure: job.failure,
      },
    )
  }

  /** Completion messages are persisted by the parent's checkpoint; restore retries delivery. */
  private async deliverClose(
    completion: Completion,
  ): Promise<void> {
    const parent = this.parentSession
    if (!parent || this.isDisposed) {
      return
    }
    if (!this.options.notifyParentOnIdle) {
      await this.options.tree.store.markDelivered(completion.id, completion.spawnedAt)
      return
    }
    this.sendCompletion(parent, completion)
  }

  /**
   * A completion the owner never received before the process ended, from the
   * store at restore.
   */
  private async deliverCompletion(record: AgentNodeRecord): Promise<void> {
    const parent = this.requireParent()
    if (!this.options.notifyParentOnIdle || record.closedPhase === null) {
      await this.options.tree.store.markDelivered(record.id, record.spawnedAt)
      return
    }
    if (record.closedAt === null) {
      throw new Error('A closed child must have a completion timestamp')
    }
    this.sendCompletion(parent, {
      id: record.id,
      spawnedAt: record.spawnedAt,
      closedAt: record.closedAt,
      description: record.description,
      metadata: record.metadata,
      phase: record.closedPhase,
      result: record.result,
      failure: record.failure,
    })
  }

  private sendCompletion(parent: AgentSession<State>, completion: Completion): void {
    void parent.acceptAgentMessage({
      id: completionMessageId(completion.id, completion.spawnedAt),
      sender: {
        id: completion.id,
        description: completion.description,
        round: completion.spawnedAt,
      },
      recipientId: this.ownerId(),
      timestamp: new Date(completion.closedAt).toISOString(),
      content: completion.phase === 'completed'
        ? completion.result ?? ''
        : completion.failure ?? '',
      event: {
        type: 'completion',
        outcome: completion.phase === 'error' ? 'failed' : completion.phase,
      },
    }, completion.metadata).catch((error: unknown) => this.reportLifecycleFailure(error))
  }

  private reportLifecycleFailure(error: unknown): void {
    // Keep the persisted child undelivered; a later open retries its lifecycle.
    this.options.tree.emit({ type: 'error', message: errorMessage(error) })
  }

  private requireParent(): AgentSession<State> {
    if (!this.parentSession) {
      throw new Error('subagent supervisor has no owner session')
    }
    return this.parentSession
  }

  /**
   * No name means the unnamed inherit profile: the parent harness, model, Host
   * and commands, always available and never configurable. A name must match a
   * declared profile; "default" is not a name.
   */
  private resolveProfile(name: string | undefined): SubagentProfile<State> {
    if (name === undefined) {
      return INHERIT_PROFILE as SubagentProfile<State>
    }
    const profile = this.options.tree.profiles?.find(
      (candidate) => candidate.name === name,
    )
    if (profile) {
      return profile
    }
    const names = this.configuredProfileNames()
    throw new Error(
      `unknown profile "${name}" (available: ${names.length > 0 ? names.join(', ') : 'none; omit --profile to inherit the parent'})`,
    )
  }

  private configuredProfileNames(): string[] {
    return (this.options.tree.profiles ?? []).map((profile) => profile.name)
  }

  private subagentPreamble(childId: string, canSpawn: boolean): string {
    return [
      `You are a subagent: a child agent session (id ${childId}) spawned by parent agent session ${this.ownerId()}. Your transcript starts empty; the task brief in the first user message is your entire context.`,
      'When you end your turn with nothing pending — no queued messages, no scheduled wakeups, no running children of your own — the session ends and your last assistant text is returned to the parent as the result. Write it for the parent agent, in the shape the task brief asked for.',
      canSpawn
        ? '`demi agent spawn` spawns your own children.'
        : 'This session may not spawn subagents.',
      "`demi agent send <id|parent>` delivers useful interim information, questions, or blockers through internal steering or an idle wakeup. It reads the message only from stdin (use a quoted heredoc). Your final answer is delivered automatically; do not send a duplicate final result. `demi agent list` renders the whole agent tree with your position.",
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
        if (block.type !== 'tool_call' || block.status === 'executing') {
          continue
        }
        const record = job.tools.find(
          (tool) => tool.toolUseId === block.toolUseId && tool.endedAt === null,
        )
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
    if (phase === 'compacting') {
      return 'compacting'
    }
    if (phase === 'idle') {
      return job.node.session.hasPendingYields() ? 'pending_yield' : 'idle'
    }
    return job.node.session.turnPhase() ?? 'provider_streaming'
  }

  private activityOf(job: ChildJob<State>, execution: SubagentExecution): string {
    const inflight = [...job.tools].reverse().find((tool) => tool.endedAt === null)
    if (execution === 'tool_executing' && inflight) {
      return inflight.title
    }
    if (execution === 'provider_streaming') {
      return 'streaming'
    }
    return execution
  }

  private executionForMs(
    job: ChildJob<State>,
    execution: SubagentExecution,
    now: number,
  ): number {
    if (execution === 'tool_executing') {
      const inflight = [...job.tools].reverse().find((tool) => tool.endedAt === null)
      if (inflight) {
        return now - inflight.startedAt
      }
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
    if (!detailed) {
      return base
    }
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
      lastAssistantTextAgoMs:
        job.lastAssistantTextAt === null ? null : now - job.lastAssistantTextAt,
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
          lines.push(
            `  [executing for ${formatDuration(now - tool.startedAt)}] ${tool.title}`,
          )
        } else {
          lines.push(
            `  [${tool.status} in ${formatDuration(tool.endedAt - tool.startedAt)}, ended ${formatDuration(now - tool.endedAt)} ago] ${tool.title}`,
          )
        }
      }
    }
    const text = boundedResultText(
      lastAssistantText(job.node.session.transcript().blocks),
    )
    if (text && job.lastAssistantTextAt !== null) {
      lines.push(
        `last assistant text (${formatDuration(now - job.lastAssistantTextAt)} ago):`,
      )
      lines.push(text)
    } else {
      lines.push('last assistant text: (none yet)')
    }
    return `${lines.join('\n')}\n`
  }

  private wireJob(
    job: ChildJob<State>,
    result?: string,
    closedAt?: number,
  ): SubagentJob {
    return {
      subagentId: job.id,
      parentSessionId: this.ownerId(),
      description: job.description,
      profile: job.profileName,
      phase: job.phase,
      startedAt: new Date(job.spawnedAt).toISOString(),
      endedAt: closedAt === undefined ? null : new Date(closedAt).toISOString(),
      metadata: job.metadata,
      ...(result !== undefined ? { result } : {}),
    }
  }
}

function trimToolRecords(tools: ChildToolRecord[]): void {
  while (tools.length > SHOW_RECENT_TOOLS) {
    const index = tools.findIndex((tool) => tool.endedAt !== null)
    if (index === -1) {
      return
    }
    tools.splice(index, 1)
  }
}

function toolCallTitle(block: Extract<Block, { type: 'tool_call' }>): string {
  try {
    const input = JSON.parse(block.input) as Record<string, unknown>
    if (typeof input.description === 'string' && input.description.trim()) {
      return input.description.trim()
    }
  } catch {
    // Fall through to the tool name.
  }
  return block.toolName
}

function lastAssistantText(blocks: Block[]): string {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i]
    if (block.type === 'text') {
      return block.text
    }
  }
  return ''
}

function boundedResultText(text: string): string {
  return utf8Slice(text, 0, SUBAGENT_RESULT_MAX_BYTES)
}
