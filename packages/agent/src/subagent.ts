import { createId, decodeUtf8, errorMessage, noop, utf8Slice } from '@demicodes/utils'
import { z } from 'zod'
import {
  BashEnvironment,
  CommandRegistry,
  type BashEnvironmentOptions,
  type Command,
  type CommandIO,
  type Host,
  type HostStore,
} from '@demicodes/shell'
import type { Block, UserContentBlock } from '@demicodes/core'
import { AgentSession } from './session'
import type { ServerFrame, SubagentJob, TranscriptPatch } from './frames'
import type {
  AgentHarness,
  AgentHarnessRuntime,
  AgentMetadata,
  AgentSessionCheckpoint,
  AgentSessionStore,
  AgentToolInvokeContext,
  SubagentProfile,
} from './types'
import { createStandardAgentTools } from './tools'
import type { AgentServerSessionOptions, PrepareShell } from './server'

/** Default per-session live-children ceiling; override with `AgentServerOptions.subagents.maxLiveSubagents`. */
export const MAX_LIVE_SUBAGENTS = 8
export const SUBAGENT_RESULT_MAX_BYTES = 32 * 1024
const SHOW_RECENT_TOOLS = 8

const SPAWN_RUNNING_HINT =
  "next: the child agent is still working; a long-running spawn is normal. shell_write steers it, shell_abort aborts it. Otherwise stop attending and end the turn — the child's completion returns as this command's result and wakes the session when it is idle. Do not poll with shell_status or timed yields; use `demi agent show` only to decide a steer or abort."

const SPAWN_PROMPT_DESCRIPTION =
  "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return."

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
  /** Supervisor of this child's own children (the grandchildren of this supervisor's owner). */
  ownSupervisor: ChildSupervisor<State>
  commandRegistry: CommandRegistry
  commandNames: string[]
  environments: Map<Host, BashEnvironment>
  pendingEnvironments: Map<Host, Promise<BashEnvironment>>
  readonlyHosts: WeakMap<Host, Host>
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
  /** Wakes the settle loop when the child set or an inbound send changes the picture. */
  wake: (() => void) | null
}

/**
 * Connection-wide flat registry of every live session in the tree: the root
 * plus every subagent at any depth. The sole basis for cross-tree addressing —
 * `send`, `steer`, `show`, and `list` resolve here, with no routing rules
 * along the tree.
 */
export class AgentDirectory<State = unknown> {
  private root: { session: AgentSession<State>; supervisor: ChildSupervisor<State> } | null = null
  private readonly entries = new Map<string, { job: ChildJob<State>; owner: ChildSupervisor<State> }>()

  attachRoot(session: AgentSession<State>, supervisor: ChildSupervisor<State>): void {
    this.root = { session, supervisor }
  }

  rootId(): string {
    if (!this.root) throw new Error('agent directory has no root session')
    return this.root.session.id()
  }

  rootSession(): AgentSession<State> {
    if (!this.root) throw new Error('agent directory has no root session')
    return this.root.session
  }

  register(job: ChildJob<State>, owner: ChildSupervisor<State>): void {
    this.entries.set(job.id, { job, owner })
  }

  unregister(id: string): void {
    this.entries.delete(id)
  }

  liveEntry(id: string): { job: ChildJob<State>; owner: ChildSupervisor<State> } | null {
    return this.entries.get(id) ?? null
  }

  /** The parent session id of a live agent; null for the root, undefined for an unknown id. */
  parentIdOf(id: string): string | null | undefined {
    if (this.root && this.root.session.id() === id) return null
    return this.entries.get(id)?.owner.ownerId()
  }

  /**
   * The whole session tree: the root, every live agent, and each live node's
   * archived children (their supervisors exist, so their archives are
   * readable). Live children order by spawn time; archived newest first.
   */
  async tree(): Promise<AgentTreeNode[]> {
    if (!this.root) return []
    const build = async (
      id: string,
      parentId: string | null,
      job: ChildJob<State> | null,
      owner: ChildSupervisor<State> | null,
      supervisor: ChildSupervisor<State>,
    ): Promise<AgentTreeNode> => {
      const liveChildren = [...this.entries.values()]
        .filter((entry) => entry.owner === supervisor)
        .sort((a, b) => a.job.spawnedAt - b.job.spawnedAt)
      const children: AgentTreeNode[] = []
      for (const entry of liveChildren) {
        children.push(await build(entry.job.id, id, entry.job, entry.owner, entry.job.ownSupervisor))
      }
      const now = Date.now()
      for (const archived of await supervisor.listArchivedJobs()) {
        children.push({
          id: archived.id,
          parentId: id,
          kind: 'archived',
          description: archived.meta.description,
          profile: archived.meta.profileName,
          phase: archived.meta.closedPhase ?? 'completed',
          closedAgoMs: archived.meta.closedAt === undefined ? null : now - archived.meta.closedAt,
          line: null,
          children: [],
        })
      }
      if (job && owner) {
        return {
          id,
          parentId,
          kind: 'live',
          description: job.description,
          profile: job.profileName,
          phase: job.phase,
          closedAgoMs: null,
          line: owner.renderListLine(job),
          children,
        }
      }
      return {
        id,
        parentId,
        kind: 'root',
        description: '',
        profile: null,
        phase: 'running',
        closedAgoMs: null,
        line: null,
        children,
      }
    }
    return [await build(this.rootId(), null, null, null, this.root.supervisor)]
  }
}

export interface AgentTreeNode {
  id: string
  parentId: string | null
  kind: 'root' | 'live' | 'archived'
  description: string
  profile: string | null
  phase: SubagentJob['phase']
  closedAgoMs: number | null
  /** Pre-rendered live status line (id, phase, ages, execution, activity); null for root/archived. */
  line: string | null
  children: AgentTreeNode[]
}

export interface ChildSupervisorOptions<State> {
  agent: AgentHarness<State>
  cwd: string
  /** Harness-configured profiles; null means the implicit `default` only. */
  profiles: SubagentProfile<State>[] | null
  /** The owner session's harness commands, before the `demi agent` injection. */
  parentCommands: Command[]
  shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  prepareShell: PrepareShell | null
  sessionOptions: AgentServerSessionOptions
  /** When false, a child closing never wakes an idle parent; the host app orchestrates the wakeup from the `subagent closed` frame. */
  notifyParentOnIdle: boolean
  /** Backing store for child checkpoints and job metadata. */
  store: HostStore
  /** Store key prefix of the owner session's directory (e.g. `agent-sessions/<id>`); children nest under `<prefix>/subagents/<childId>`. */
  storePrefix: string
  /** Connection-wide registry shared by every supervisor in the tree. */
  directory: AgentDirectory<State>
  /** Live-children ceiling for this supervisor (from `AgentServerOptions.subagents.maxLiveSubagents`). */
  maxLiveSubagents: number
  /** Invoked whenever the live-children set changes; wired to the owning job's settle loop. */
  onJobsChanged: (() => void) | null
  emit(frame: ServerFrame): void
}

/**
 * The on-store shape of a child (`<prefix>/subagents/<id>/job.json`); its
 * checkpoint sits beside it. Without `closedPhase` the child is live (a parent
 * restore resumes it); with `closedPhase` it is archived — finished but
 * revivable with `demi agent resume`, skipped by restore, never pruned: it
 * lives exactly as long as the owning session directory.
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
 * Per-session subagent supervisor. Every session — root or subagent — owns one
 * and carries the identical `demi agent` command tree, so spawn nests to any
 * depth. The supervisor owns its direct children's lifecycle (spawn / abort /
 * resume / natural end), their shell environments, and the `subagent*`
 * protocol frames; communication and reads (`send` / `steer` / `show` /
 * `list`) resolve through the shared AgentDirectory and reach any live agent
 * in the tree. Children persist under the owner's session directory,
 * recursively, and reopening a session restores its whole subtree.
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

  ownerId(): string {
    if (!this.parentSession) throw new Error('subagent supervisor has no owner session')
    return this.parentSession.id()
  }

  hasLiveJobs(): boolean {
    return this.jobs.size > 0
  }

  /** The `agent` node AgentServer (and every child assembly) grafts under the registry's `demi` root. */
  rootCommandNode(): Command {
    const profileNames = this.configuredProfileNames()
    return {
      name: 'agent',
      summary:
        'Start an isolated child agent session and wait for its result. The command stays running until the child session ends; stdout is the child\'s last assistant text. While it is the foreground job, shell_write steers the child and shell_abort aborts it. Run several in separate shell_exec calls with short timeoutMs to fan out, then end the turn — completion wakes an idle session; do not poll. Children can spawn children of their own.',
      successOutput:
        'first stderr line is "subagentId: <id>" at start; stdout is the child\'s last assistant text (empty is valid), written only at exit',
      failureOutput: 'non-zero exit with the abort or failure reason on stderr',
      input: {
        prompt: z.string().optional().describe(SPAWN_PROMPT_DESCRIPTION),
        profile: z
          .string()
          .optional()
          .describe(`Named subagent profile configured at harness assembly. Available: ${profileNames.join(', ')}.`),
        description: z
          .string()
          .optional()
          .describe('Short UI title distinguishing concurrent children.'),
      },
      positionals: ['prompt'],
      stdinField: 'prompt',
      output: { json: z.object({ subagentId: z.string(), text: z.string() }) },
      runningHint: SPAWN_RUNNING_HINT,
      run: async ({ parsed, io, signal, stdinStream }) => {
        const prompt = String(parsed.values.prompt ?? '').trim()
        if (!prompt) {
          await io.stderr('demi agent: prompt must not be empty\n')
          return { exitCode: 1 }
        }
        let job: ChildJob<State>
        try {
          job = await this.spawn({
            prompt,
            profileName: parsed.values.profile === undefined ? undefined : String(parsed.values.profile),
            description: parsed.values.description === undefined ? '' : String(parsed.values.description),
          })
        } catch (error) {
          await io.stderr(`demi agent: ${errorMessage(error)}\n`)
          return { exitCode: 1 }
        }
        return this.attendChild(job, { io, isJson: parsed.json === true, signal, stdinStream })
      },
      subcommands: [
        {
          name: 'send',
          summary:
            'Leave a message for any live agent in the tree (`demi agent list`), or `parent`. The target sees it as a new user turn at its next turn boundary, never mid-turn; a message to a finishing subagent extends its life by one turn. Fire-and-forget: queues and returns, never waits. An archived target fails — only its parent can revive it with resume.',
          input: {
            id: z.string().describe('Target agent id from the tree, or "parent" for the session that spawned this one'),
            message: z.string().optional().describe('Message body; positional, or stdin/heredoc when omitted.'),
          },
          positionals: ['id', 'message'],
          stdinField: 'message',
          output: { json: z.object({ id: z.string(), accepted: z.boolean() }) },
          run: async ({ parsed, io }) => {
            const message = String(parsed.values.message ?? '').trim()
            if (!message) {
              await io.stderr('demi agent send: message must not be empty\n')
              return { exitCode: 1 }
            }
            try {
              const targetId = this.deliverSend(String(parsed.values.id), message)
              await io.stdout(parsed.json ? `${JSON.stringify({ id: targetId, accepted: true })}\n` : `sent to ${targetId}\n`)
              return { exitCode: 0 }
            } catch (error) {
              await io.stderr(`demi agent send: ${errorMessage(error)}\n`)
              return { exitCode: 1 }
            }
          },
        },
        {
          name: 'steer',
          summary:
            "Chime into a running agent's current turn: the target sees the message at its next sampling/tool boundary and continues its current work with the new information. Nothing is cancelled and the turn does not restart. Fails when the target has no running turn — use send for that. Targets any live agent in the tree, or `parent`.",
          input: {
            id: z.string().describe('Target agent id from the tree, or "parent" for the session that spawned this one'),
            message: z.string().optional().describe('Message body; positional, or stdin/heredoc when omitted.'),
          },
          positionals: ['id', 'message'],
          stdinField: 'message',
          output: { json: z.object({ id: z.string(), accepted: z.boolean() }) },
          run: async ({ parsed, io }) => {
            const message = String(parsed.values.message ?? '').trim()
            if (!message) {
              await io.stderr('demi agent steer: message must not be empty\n')
              return { exitCode: 1 }
            }
            try {
              const targetId = await this.deliverSteer(String(parsed.values.id), message)
              await io.stdout(parsed.json ? `${JSON.stringify({ id: targetId, accepted: true })}\n` : `steered ${targetId}\n`)
              return { exitCode: 0 }
            } catch (error) {
              await io.stderr(`demi agent steer: ${errorMessage(error)}\n`)
              return { exitCode: 1 }
            }
          },
        },
        {
          name: 'abort',
          summary: 'Abort one of your own running children and its whole subtree. Siblings are untouched; only the spawning session may abort a child.',
          input: { id: z.string().describe('subagentId of one of your running children') },
          positionals: ['id'],
          output: { json: z.object({ id: z.string(), aborted: z.boolean() }) },
          run: async ({ parsed, io }) => {
            const id = String(parsed.values.id)
            if (!this.jobs.has(id)) {
              await io.stderr(`demi agent abort: "${id}" is not one of your running children\n`)
              return { exitCode: 1 }
            }
            await this.abortSubtree(id)
            await io.stdout(parsed.json ? `${JSON.stringify({ id, aborted: true })}\n` : `aborted ${id}\n`)
            return { exitCode: 0 }
          },
        },
        {
          name: 'resume',
          summary:
            'Revive one of your own archived (finished) children with a new user message on top of its preserved transcript. Behaves like the spawn command afterwards: stays running until the child ends again, stdout is its new last assistant text, shell_write steers, shell_abort aborts. Archived ids are in `demi agent list`; only the spawning session may resume its child.',
          input: {
            id: z.string().describe('subagentId of one of your archived children'),
            message: z.string().optional().describe('The reviving user message; positional, or stdin/heredoc when omitted.'),
          },
          positionals: ['id', 'message'],
          stdinField: 'message',
          output: { json: z.object({ subagentId: z.string(), text: z.string() }) },
          runningHint: SPAWN_RUNNING_HINT,
          run: async ({ parsed, io, signal, stdinStream }) => {
            const id = String(parsed.values.id)
            const message = String(parsed.values.message ?? '').trim()
            if (!message) {
              await io.stderr('demi agent resume: message must not be empty\n')
              return { exitCode: 1 }
            }
            let job: ChildJob<State>
            try {
              job = await this.resumeArchived(id, message)
            } catch (error) {
              await io.stderr(`demi agent resume: ${errorMessage(error)}\n`)
              return { exitCode: 1 }
            }
            return this.attendChild(job, { io, isJson: parsed.json === true, signal, stdinStream })
          },
        },
        {
          name: 'list',
          summary:
            'Render the whole session tree from the root down, marking your own position. Live agents show phase, ages, execution, and activity; each node\'s archived (finished, revivable by its parent) children render beneath it. Every age is relative to now. A read, not a wait — not for polling loops.',
          output: { json: z.object({ tree: z.array(z.unknown()) }) },
          run: async ({ parsed, io }) => {
            const nodes = await this.options.directory.tree()
            if (parsed.json) {
              await io.stdout(`${JSON.stringify({ tree: flattenTree(nodes, this.ownerId()) })}\n`)
              return { exitCode: 0 }
            }
            const lines: string[] = []
            for (const node of nodes) renderTreeNode(node, '', true, this.ownerId(), lines)
            await io.stdout(`${lines.join('\n')}\n`)
            return { exitCode: 0 }
          },
        },
        {
          name: 'show',
          summary:
            'Bounded snapshot of any live agent in the tree (root excluded): execution state, recent tool titles with durations, last assistant text. Every duration is relative to now — use the ages to tell motion from stall. Omits tool outputs, file contents, and older turns. A read, not a wait — not for polling loops.',
          input: { id: z.string().describe('Agent id from the tree') },
          positionals: ['id'],
          output: { json: z.object({ agent: z.unknown() }) },
          run: async ({ parsed, io }) => {
            const id = String(parsed.values.id)
            const entry = this.options.directory.liveEntry(id)
            if (!entry) {
              await io.stderr(`demi agent show: no live agent "${id}"\n`)
              return { exitCode: 1 }
            }
            if (parsed.json) await io.stdout(`${JSON.stringify({ agent: entry.owner.snapshot(entry.job, true) })}\n`)
            else await io.stdout(entry.owner.renderShow(entry.job))
            return { exitCode: 0 }
          },
        },
      ],
    }
  }

  hasShell(shellId: string): boolean {
    return this.environmentScopeForShell(shellId) !== null
  }

  /** Resolves the descendant scope owning a shell (recursively), for the command bridge dispatch. */
  environmentScopeForShell(
    shellId: string,
  ): { environment: BashEnvironment; commandNames: ReadonlySet<string>; agentSessionId: string } | null {
    for (const job of this.jobs.values()) {
      for (const environment of job.environments.values()) {
        if (environment.getShell(shellId)) {
          return { environment, commandNames: new Set(job.commandNames), agentSessionId: job.id }
        }
      }
      const nested = job.ownSupervisor.environmentScopeForShell(shellId)
      if (nested) return nested
    }
    return null
  }

  /** Re-emits `subagent started` + transcript reset for the whole live subtree (transcript resync). */
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
      job.ownSupervisor.replay()
    }
  }

  /**
   * Detaches the live subtree on connection teardown: aborts in-flight turns,
   * flushes checkpoints, and keeps the persisted jobs so the next open of the
   * owner restores them — the same dispose semantics as the owner session.
   * No `closed` frame is emitted: the children are not done, just paused.
   */
  async dispose(): Promise<void> {
    this.isDisposed = true
    for (const job of [...this.jobs.values()]) {
      job.isClosing = true
      job.unsubscribe()
      this.jobs.delete(job.id)
      this.options.directory.unregister(job.id)
      await job.ownSupervisor.dispose()
      await job.session.dispose().catch(noop)
      await this.disposeJobShells(job)
      job.settleClosed({ phase: 'aborted' })
      job.wake?.()
    }
  }

  /**
   * Rebuilds every persisted live child of this owner and finishes what it was
   * doing: an interrupted turn resumes from its resume point; an already
   * quiescent child closes with its result. Recursive: each restored child
   * restores its own subtree. Children share the owner's persistence
   * lifecycle — a session restore is a subtree restore.
   */
  async restore(): Promise<void> {
    if (!this.parentSession || this.isDisposed) return
    const prefix = `${this.options.storePrefix}/subagents/`
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
    if (!this.parentSession) return
    const meta = await this.options.store.readJson<PersistedSubagentJob>(this.childStoreKey(id, 'job.json'))
    // Archived children are finished: only `demi agent resume` revives them.
    if (meta?.closedPhase) return
    const checkpoint = await this.childSessionStore(id).loadCheckpoint()
    if (!meta || !checkpoint) throw new Error('incomplete persisted subagent')
    const job = this.reassembleJob(id, meta, checkpoint)
    // A live persisted job is by definition unfinished (closeJob archives it),
    // so always resume: findResumePoint decides how far to unwind, exactly like
    // an interrupted parent session. A child that had already produced its final
    // text re-infers one turn and closes through the normal quiescence path.
    this.trackTurn(job, job.session.resume(meta.metadata ? { metadata: meta.metadata } : {}))
    void this.settleJob(job)
    await job.ownSupervisor.restore()
  }

  /** Shared by restore and resume: rebuild a persisted child's job and session from its checkpoint. */
  private reassembleJob(id: string, meta: PersistedSubagentJob, checkpoint: AgentSessionCheckpoint<State>): ChildJob<State> {
    const parent = this.parentSession
    if (!parent) throw new Error('subagent supervisor has no owner session')
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
    if (!parent) throw new Error('subagent supervisor has no owner session')
    if (this.isDisposed) throw new Error('owner session is closing')
    if (this.jobs.has(id)) throw new Error(`subagent "${id}" is still running; send or steer it instead`)
    if (this.jobs.size >= this.options.maxLiveSubagents) {
      throw new Error(`at most ${this.options.maxLiveSubagents} running subagents per session; abort one or wait for a result`)
    }
    const meta = await this.options.store.readJson<PersistedSubagentJob>(this.childStoreKey(id, 'job.json'))
    if (!meta?.closedPhase) throw new Error(`no archived subagent "${id}" (see \`demi agent list\`)`)
    const checkpoint = await this.childSessionStore(id).loadCheckpoint()
    if (!checkpoint) throw new Error(`archived subagent "${id}" has no checkpoint left`)
    const liveMeta: PersistedSubagentJob = {
      description: meta.description,
      profileName: meta.profileName,
      metadata: parent.actionMetadata(),
      spawnedAt: Date.now(),
    }
    await this.options.store.writeJson(this.childStoreKey(id, 'job.json'), liveMeta)
    const job = this.reassembleJob(id, liveMeta, checkpoint)
    this.trackTurn(job, job.session.send([{ type: 'text', text: message }], liveMeta.metadata ? { metadata: liveMeta.metadata } : {}))
    void this.settleJob(job)
    return job
  }

  /** Every archived (finished, revivable) child of this owner, newest first. */
  async listArchivedJobs(): Promise<{ id: string; meta: PersistedSubagentJob }[]> {
    if (!this.parentSession) return []
    const prefix = `${this.options.storePrefix}/subagents/`
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
  }): Promise<ChildJob<State>> {
    const parent = this.parentSession
    if (!parent) throw new Error('subagent supervisor has no owner session')
    if (this.isDisposed) throw new Error('owner session is closing')
    if (this.jobs.size >= this.options.maxLiveSubagents) {
      throw new Error(`at most ${this.options.maxLiveSubagents} running subagents per session; abort one or wait for a result`)
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
    this.trackTurn(job, session.send([{ type: 'text', text: input.prompt }], metadata ? { metadata } : {}))
    void this.settleJob(job)
    return job
  }

  /**
   * Everything spawn and restore share: the command tree (identical to the
   * owner's — children spawn, message, and observe exactly like any session),
   * the child's own supervisor, the harness runtime, and the job record
   * (session attached separately).
   */
  private assembleJob(input: {
    id: string
    description: string
    profileName: string | null
    profile: SubagentProfile<State>
    metadata: AgentMetadata | null
    spawnedAt: number
  }): { job: ChildJob<State>; runtime: AgentHarnessRuntime<State> } {
    const { id, profile } = input
    const inherited = profile.commands ? profile.commands([...this.options.parentCommands]) : [...this.options.parentCommands]

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
      ownSupervisor: null as unknown as ChildSupervisor<State>,
      commandRegistry: new CommandRegistry(),
      commandNames: [],
      environments: new Map(),
      pendingEnvironments: new Map(),
      readonlyHosts: new WeakMap(),
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
      wake: null,
    }

    job.ownSupervisor = new ChildSupervisor<State>({
      ...this.options,
      parentCommands: inherited,
      storePrefix: `${this.options.storePrefix}/subagents/${id}`,
      onJobsChanged: () => job.wake?.(),
    })
    const commands = injectSubagentCommand(inherited, job.ownSupervisor.rootCommandNode())
    for (const command of commands) job.commandRegistry.register(command)
    job.commandNames = job.commandRegistry.list().map((command) => command.name)
    const commandsPrompt = job.commandRegistry.renderHelp()

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
    job.ownSupervisor.attachParent(session)
    job.unsubscribe = session.subscribe((event) => {
      if (event.type === 'phase_changed') {
        job.wake?.()
        return
      }
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
    this.options.directory.register(job, this)
    this.options.onJobsChanged?.()
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
    return `${this.options.storePrefix}/subagents/${childId}/${file}`
  }

  private childSessionStore(childId: string): AgentSessionStore<State> {
    return {
      saveCheckpoint: (checkpoint) => this.options.store.writeJson(this.childStoreKey(childId, 'checkpoint.json'), checkpoint),
      loadCheckpoint: () => this.options.store.readJson(this.childStoreKey(childId, 'checkpoint.json')),
    }
  }

  private async deletePersistedJob(id: string): Promise<void> {
    await this.options.store.delete(this.childStoreKey(id, 'checkpoint.json')).catch(noop)
    await this.options.store.delete(this.childStoreKey(id, 'job.json')).catch(noop)
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
    const selfId = this.ownerId()
    let id = rawId
    if (id === 'parent') {
      const parentId = this.options.directory.parentIdOf(selfId)
      if (parentId === null) throw new Error('the root session has no parent')
      if (parentId === undefined) throw new Error('this session is not in the agent directory')
      id = parentId
    }
    if (id === selfId) throw new Error('cannot message your own session')
    if (id === this.options.directory.rootId()) {
      return { id, session: this.options.directory.rootSession(), job: null, owner: null }
    }
    const entry = this.options.directory.liveEntry(id)
    if (!entry || entry.job.isClosing) {
      throw new Error(`no live agent "${id}" (see \`demi agent list\`; an archived child is revived only by its parent via resume)`)
    }
    return { id, session: entry.job.session, job: entry.job, owner: entry.owner }
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
    const entry = this.options.directory.liveEntry(selfId)
    const description = entry ? entry.job.description : 'root session'
    return `[agent ${selfId}${description ? ` — ${description}` : ''}]`
  }

  /** Metadata for a turn on the root: the sender subtree's spawning round, or null from the root itself. */
  private senderMetadata(): AgentMetadata | null {
    return this.options.directory.liveEntry(this.ownerId())?.job.metadata ?? null
  }

  /** Delivers one stdin chunk from the attending spawner: mid-turn as a steer, otherwise as a new user turn. */
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
    this.trackTurn(job, job.session.send(content))
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
   * The one place a child closes naturally. Loops until the child is
   * quiescent: no running or queued turn (the session action queue doubles as
   * the mailbox), no pending yield wakeups, and no live children of its own.
   * The final check-and-close is synchronous, so a send that lands before it
   * is processed and one that lands after it fails on `isClosing` — nothing
   * drops silently.
   */
  private async settleJob(job: ChildJob<State>): Promise<void> {
    while (!job.isClosing) {
      if (job.session.isSettled() && !job.session.hasPendingYields() && !job.ownSupervisor.hasLiveJobs()) {
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
        if (job.session.isSettled()) return
        void job.session.waitUntilDone().then(finish)
      })
    }
  }

  private async closeJob(job: ChildJob<State>, phase: SubagentClose['phase']): Promise<void> {
    if (job.isClosing) return
    job.isClosing = true
    job.phase = phase
    job.unsubscribe()
    this.jobs.delete(job.id)
    this.options.directory.unregister(job.id)
    job.wake?.()

    // A natural completion has no live descendants by construction; an abort
    // or error tears the subtree down with it.
    await job.ownSupervisor.abortAll()

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

    const close: SubagentClose = {
      phase,
      ...(result !== undefined ? { result } : {}),
      ...(job.failure ? { failure: job.failure } : {}),
    }
    this.options.emit({ type: 'subagent', event: 'closed', job: this.wireJob(job, result) })
    job.settleClosed(close)
    this.options.onJobsChanged?.()
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

  /** Wakes an idle owner with a user send; an owner blocked in the spawn gets the tool result instead. */
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

  private async childEnvironment(
    job: ChildJob<State>,
    ctx: Pick<AgentToolInvokeContext<State>, 'state' | 'metadata'>,
  ): Promise<BashEnvironment> {
    const resolved = await this.options.agent.host({
      agentSessionId: job.id,
      state: ctx.state,
      cwd: this.options.cwd,
      metadata: ctx.metadata,
    })
    let host = resolved
    if (job.profile.readonly) {
      const wrapped = job.readonlyHosts.get(resolved) ?? createReadonlyHost(resolved)
      job.readonlyHosts.set(resolved, wrapped)
      host = wrapped
    }
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

  private async createChildEnvironment(job: ChildJob<State>, host: Host): Promise<BashEnvironment> {
    // A read-only child cannot spawn processes, so bridge shims are useless
    // (and un-materializable on a write-rejecting Host): skip prepareShell.
    const prepared =
      this.options.prepareShell && !job.profile.readonly
        ? await this.options.prepareShell({
            agentSessionId: job.id,
            host,
            commandNames: job.commandNames,
            shell: this.options.shellOptions,
          })
        : this.options.shellOptions
    return new BashEnvironment({
      ...prepared,
      initialEnv: {
        ...prepared.initialEnv,
        DEMI_SUBAGENT_ID: job.id,
        DEMI_PARENT_SESSION_ID: this.ownerId(),
      },
      host,
      commands: job.commandRegistry,
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
      `You are a subagent: an isolated child agent session (id ${childId}) spawned by parent agent session ${this.ownerId()}. Your transcript starts empty; the task brief in the first user message is your entire context.`,
      'When you end your turn with nothing pending — no queued messages, no scheduled wakeups, no running children of your own — the session ends and your last assistant text is returned to the parent as the result. Write it for the parent agent, in the shape the task brief asked for.',
      '`demi agent` spawns your own children. `demi agent send <id|parent> <message>` leaves a message any live agent sees at its next turn boundary; `demi agent steer <id> <message>` chimes into a running agent\'s current turn; `demi agent list` renders the whole agent tree with your position.',
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

  renderListLine(job: ChildJob<State>): string {
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

  renderShow(job: ChildJob<State>): string {
    const now = Date.now()
    const execution = this.executionOf(job)
    const lines = [
      `id: ${job.id}`,
      `parent: ${this.ownerId()}`,
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
      parentSessionId: this.ownerId(),
      description: job.description,
      profile: job.profileName,
      phase: job.phase,
      metadata: job.metadata,
      ...(result !== undefined ? { result } : {}),
    }
  }
}

function renderTreeNode(node: AgentTreeNode, prefix: string, isLast: boolean, selfId: string, lines: string[]): void {
  const marker = node.id === selfId ? ' ← you' : ''
  const body =
    node.kind === 'root'
      ? `● ${node.id}  (root session)${marker}`
      : node.kind === 'archived'
        ? `○ ${node.id}  archived (${node.phase}${node.closedAgoMs === null ? '' : ` ${formatDuration(node.closedAgoMs)} ago`})  ${node.description ? `"${node.description}"` : '(no description)'}`
        : `● ${node.line}${marker}`
  if (node.parentId === null) lines.push(body)
  else lines.push(`${prefix}${isLast ? '└─' : '├─'}${body}`)
  const childPrefix = node.parentId === null ? '' : `${prefix}${isLast ? '  ' : '│ '}`
  node.children.forEach((child, index) => {
    renderTreeNode(child, childPrefix, index === node.children.length - 1, selfId, lines)
  })
}

function flattenTree(nodes: AgentTreeNode[], selfId: string): Record<string, unknown>[] {
  const flat: Record<string, unknown>[] = []
  const visit = (node: AgentTreeNode): void => {
    flat.push({
      subagentId: node.id,
      parentSessionId: node.parentId,
      kind: node.kind,
      description: node.description,
      profile: node.profile,
      phase: node.phase,
      closedAgoMs: node.closedAgoMs,
      self: node.id === selfId,
    })
    node.children.forEach(visit)
  }
  nodes.forEach(visit)
  return flat
}

/**
 * Grafts the `agent` node under a `demi` root: onto an existing harness `demi`
 * tree, or as a new `demi` root when the harness has none.
 */
export function injectSubagentCommand(commands: Command[], agentNode: Command): Command[] {
  const demiIndex = commands.findIndex((command) => command.name === 'demi')
  if (demiIndex === -1) {
    return [...commands, { name: 'demi', summary: 'Demi agent runtime commands.', subcommands: [agentNode] }]
  }
  const demi = commands[demiIndex]!
  const subcommands = [...(demi.subcommands ?? []).filter((command) => command.name !== 'agent'), agentNode]
  const next = [...commands]
  next[demiIndex] = { ...demi, subcommands }
  return next
}

/**
 * Host wrapper for read-only subagent profiles: filesystem mutation is
 * rejected outside the shell's own command-artifacts tree, and process spawn
 * is rejected outright (a real process cannot be write-restricted).
 *
 * Every facet delegates method by method: Host facets are class instances
 * whose methods live on the prototype, so object spread would drop them.
 */
export function createReadonlyHost(host: Host): Host {
  const artifactsRoot = host.commandArtifactsDir.replace(/\/+$/, '')
  const isArtifactPath = (path: string, cwd: string | undefined): boolean => {
    const resolved = path.startsWith('/') ? path : `${(cwd ?? host.defaultCwd).replace(/\/+$/, '')}/${path}`
    return resolved === artifactsRoot || resolved.startsWith(`${artifactsRoot}/`)
  }
  const deny = (operation: string): Promise<never> =>
    Promise.reject(new Error(`read-only subagent: ${operation} is not permitted on this Host`))
  const readonlyFs: Host['fs'] = {
    readFile: (path, options) => host.fs.readFile(path, options),
    exists: (path, options) => host.fs.exists(path, options),
    stat: (path, options) => host.fs.stat(path, options),
    lstat: (path, options) => host.fs.lstat(path, options),
    readdir: ((path: string, options?: { cwd?: string; withFileTypes?: boolean }) =>
      host.fs.readdir(path, options as { cwd?: string; withFileTypes: true })) as Host['fs']['readdir'],
    readlink: (path, options) => host.fs.readlink(path, options),
    realpath: (path, options) => host.fs.realpath(path, options),
    writeFile: (path, data, options) =>
      isArtifactPath(path, options?.cwd) ? host.fs.writeFile(path, data, options) : deny(`write ${path}`),
    appendFile: (path, data, options) =>
      isArtifactPath(path, options?.cwd) ? host.fs.appendFile(path, data, options) : deny(`append ${path}`),
    mkdir: (path, options) =>
      isArtifactPath(path, options?.cwd) ? host.fs.mkdir(path, options) : deny(`mkdir ${path}`),
    rm: (path) => deny(`rm ${path}`),
    cp: (path, destination, options) =>
      isArtifactPath(destination, options?.cwd) ? host.fs.cp(path, destination, options) : deny(`cp to ${destination}`),
    mv: (path, destination) => deny(`mv ${path} ${destination}`),
    chmod: (path) => deny(`chmod ${path}`),
    symlink: (_target, path) => deny(`symlink ${path}`),
    link: (_existingPath, path) => deny(`link ${path}`),
    utimes: (path) => deny(`utimes ${path}`),
  }
  return {
    defaultCwd: host.defaultCwd,
    commandArtifactsDir: host.commandArtifactsDir,
    identity: host.identity,
    store: host.store,
    fs: readonlyFs,
    process: {
      openCwd: (path) => host.process.openCwd(path),
      spawn: () => deny('process spawn'),
    },
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

function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000))
  if (totalSeconds < 60) return `${totalSeconds}s`
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  if (minutes < 60) return seconds > 0 ? `${minutes}m${seconds}s` : `${minutes}m`
  const hours = Math.floor(minutes / 60)
  const remMinutes = minutes % 60
  return remMinutes > 0 ? `${hours}h${remMinutes}m` : `${hours}h`
}
