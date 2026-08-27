import { createId, decodeUtf8, errorMessage, noop, utf8Slice } from '@demicodes/utils'
import { z } from 'zod'
import {
  BashEnvironment,
  CommandRegistry,
  type BashEnvironmentOptions,
  type Command,
  type Host,
} from '@demicodes/shell'
import type { Block, UserContentBlock } from '@demicodes/core'
import { AgentSession } from './session'
import type { ServerFrame, SubagentJob, TranscriptPatch } from './frames'
import type {
  AgentHarness,
  AgentHarnessRuntime,
  AgentMetadata,
  AgentToolInvokeContext,
  SubagentProfile,
} from './types'
import { createStandardAgentTools } from './tools'
import type { AgentServerSessionOptions, PrepareShell } from './server'

export const MAX_LIVE_SUBAGENTS = 8
export const SUBAGENT_RESULT_MAX_BYTES = 32 * 1024
const SHOW_RECENT_TOOLS = 8

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
}

export interface ChildSupervisorOptions<State> {
  agent: AgentHarness<State>
  cwd: string
  /** Harness-configured profiles; null means the implicit `default` only. */
  profiles: SubagentProfile<State>[] | null
  /** Parent registered commands (harness list, before the `demi agent` injection). */
  parentCommands: Command[]
  shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  prepareShell: PrepareShell | null
  sessionOptions: AgentServerSessionOptions
  /** When false, a child closing never wakes an idle parent; the host app orchestrates the wakeup from the `subagent closed` frame. */
  notifyParentOnIdle: boolean
  emit(frame: ServerFrame): void
}

/**
 * Per-parent subagent supervisor. Owns every child `AgentSession` this parent
 * spawns: lifecycle (spawn / steer / abort / natural end), the `demi agent`
 * command tree, child shell environments, and the `subagent*` protocol frames
 * on the parent connection. Children are in-memory only: no store, no
 * checkpoint, gone with the parent.
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
  rootCommandNode(): Command {
    const profileNames = this.configuredProfileNames()
    return {
      name: 'agent',
      summary:
        'Start an isolated child agent session and wait for its result. The command stays running until the child session ends; stdout is the child\'s last assistant text. While it is the foreground job, shell_write steers the child and shell_abort aborts it. Run several in separate shell_exec calls with short timeoutMs to fan out.',
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
          if (parsed.json) await io.stdout(`${JSON.stringify({ subagentId: job.id, text })}\n`)
          else if (text.length > 0) await io.stdout(text.endsWith('\n') ? text : `${text}\n`)
          return { exitCode: 0 }
        }
        if (close.phase === 'aborted') {
          await io.stderr(`demi agent: subagent ${job.id} aborted\n`)
          return { exitCode: 130 }
        }
        await io.stderr(`demi agent: subagent ${job.id} failed: ${close.failure ?? 'unknown error'}\n`)
        return { exitCode: 1 }
      },
      subcommands: [
        {
          name: 'steer',
          summary: 'Send a user steer to a running child. Queues until the child can take it; does not wait.',
          input: {
            id: z.string().describe('subagentId from spawn stderr'),
            message: z.string().optional().describe('Message body; positional, or stdin/heredoc when omitted.'),
          },
          positionals: ['id', 'message'],
          stdinField: 'message',
          output: { json: z.object({ id: z.string(), accepted: z.boolean() }) },
          run: async ({ parsed, io }) => {
            const id = String(parsed.values.id)
            const message = String(parsed.values.message ?? '').trim()
            if (!message) {
              await io.stderr('demi agent steer: message must not be empty\n')
              return { exitCode: 1 }
            }
            const job = this.jobs.get(id)
            if (!job) {
              await io.stderr(`demi agent steer: no running subagent "${id}"\n`)
              return { exitCode: 1 }
            }
            await this.steerChild(job, message)
            await io.stdout(parsed.json ? `${JSON.stringify({ id, accepted: true })}\n` : `steered ${id}\n`)
            return { exitCode: 0 }
          },
        },
        {
          name: 'abort',
          summary: 'Abort a running child and its subtree. Siblings are untouched.',
          input: { id: z.string().describe('subagentId from spawn stderr') },
          positionals: ['id'],
          output: { json: z.object({ id: z.string(), aborted: z.boolean() }) },
          run: async ({ parsed, io }) => {
            const id = String(parsed.values.id)
            if (!this.jobs.has(id)) {
              await io.stderr(`demi agent abort: no running subagent "${id}"\n`)
              return { exitCode: 1 }
            }
            await this.abortSubtree(id)
            await io.stdout(parsed.json ? `${JSON.stringify({ id, aborted: true })}\n` : `aborted ${id}\n`)
            return { exitCode: 0 }
          },
        },
        {
          name: 'list',
          summary:
            'Snapshot roster of this session\'s running children (finished children are absent). One line per child with ages relative to now. A read, not a wait — not for polling loops.',
          output: { json: z.object({ agents: z.array(z.unknown()) }) },
          run: async ({ parsed, io }) => {
            const jobs = [...this.jobs.values()]
            if (parsed.json) {
              await io.stdout(`${JSON.stringify({ agents: jobs.map((job) => this.snapshot(job, false)) })}\n`)
              return { exitCode: 0 }
            }
            if (jobs.length === 0) {
              await io.stdout('no running subagents\n')
              return { exitCode: 0 }
            }
            for (const job of jobs) await io.stdout(`${this.renderListLine(job)}\n`)
            return { exitCode: 0 }
          },
        },
        {
          name: 'show',
          summary:
            'Bounded snapshot of one running child: execution state, recent tool titles with durations, last assistant text. Every duration is relative to now — use the ages to tell motion from stall. Omits tool outputs, file contents, and older turns. A read, not a wait — not for polling loops.',
          input: { id: z.string().describe('subagentId from spawn stderr') },
          positionals: ['id'],
          output: { json: z.object({ agent: z.unknown() }) },
          run: async ({ parsed, io }) => {
            const id = String(parsed.values.id)
            const job = this.jobs.get(id)
            if (!job) {
              await io.stderr(`demi agent show: no running subagent "${id}"\n`)
              return { exitCode: 1 }
            }
            if (parsed.json) await io.stdout(`${JSON.stringify({ agent: this.snapshot(job, true) })}\n`)
            else await io.stdout(this.renderShow(job))
            return { exitCode: 0 }
          },
        },
      ],
    }
  }

  hasShell(shellId: string): boolean {
    return this.environmentScopeForShell(shellId) !== null
  }

  /** Resolves the child scope owning a shell, for the command bridge dispatch. */
  environmentScopeForShell(
    shellId: string,
  ): { environment: BashEnvironment; commandNames: ReadonlySet<string>; agentSessionId: string } | null {
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

  async dispose(): Promise<void> {
    this.isDisposed = true
    for (const job of [...this.jobs.values()]) {
      await this.closeJob(job, 'aborted')
    }
  }

  async abortSubtree(id: string): Promise<void> {
    const job = this.jobs.get(id)
    if (!job) return
    // Depth is 1: the subtree is the node itself. Descendants would close here first.
    await this.closeJob(job, 'aborted')
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

    const agentNode = this.createChildAgentNode(id, input.description)
    const inherited = profile.commands ? profile.commands([...this.options.parentCommands]) : [...this.options.parentCommands]
    const commands = injectSubagentCommand(inherited, agentNode)
    const commandRegistry = new CommandRegistry()
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
      profileName: input.profileName ?? (this.options.profiles ? profile.name : null),
      metadata,
      session: null as unknown as AgentSession<State>,
      commandRegistry,
      commandNames,
      environments: new Map(),
      pendingEnvironments: new Map(),
      readonlyHosts: new WeakMap(),
      spawnedAt: Date.now(),
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

    job.session = new AgentSession<State>(
      {
        provider: parent.cloneProviderRuntime(),
        model: profile.model ?? structuredClone(parent.modelSelection),
        cwd: this.options.cwd,
        runtime,
        state: agent.initialState(),
      },
      { agentSessionId: id, ...this.options.sessionOptions },
    )
    job.unsubscribe = job.session.subscribe((event) => {
      if (event.type !== 'transcript_changed') return
      this.recordTelemetry(job, event.patches)
      this.options.emit({
        type: 'subagent_transcript_patch',
        subagentId: id,
        patches: event.patches,
        revision: event.revision,
      })
    })
    this.jobs.set(id, job)

    this.options.emit({ type: 'subagent', event: 'started', job: this.wireJob(job) })
    this.options.emit({ type: 'subagent_transcript_reset', subagentId: id, blocks: [], revision: 0 })
    this.watchChild(job, input.prompt)
    return job
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
    await job.session.dispose().catch(noop)
    const environments = new Set([...job.environments.values()])
    for (const pending of job.pendingEnvironments.values()) {
      const environment = await pending.catch(() => null)
      if (environment) environments.add(environment)
    }
    for (const environment of environments) await environment.disposeAllShells().catch(noop)

    const close: SubagentClose = {
      phase,
      ...(result !== undefined ? { result } : {}),
      ...(job.failure ? { failure: job.failure } : {}),
    }
    this.options.emit({ type: 'subagent', event: 'closed', job: this.wireJob(job, result) })
    job.settleClosed(close)
    this.notifyIdleParent(job, close)
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

  private createChildAgentNode(childId: string, description: string): Command {
    return {
      name: 'agent',
      summary: 'Subagent bridge to the parent session.',
      subcommands: [
        {
          name: 'send-parent',
          summary:
            'Send an interim user message to the parent session. The parent sees it only when it is not blocked waiting on this session. Your result is still the last assistant text when this session ends, not this message.',
          input: {
            message: z.string().optional().describe('Message body; positional, or stdin/heredoc when omitted.'),
          },
          positionals: ['message'],
          stdinField: 'message',
          output: { json: z.object({ accepted: z.boolean() }) },
          run: async ({ parsed, io }) => {
            const message = String(parsed.values.message ?? '').trim()
            if (!message) {
              await io.stderr('demi agent send-parent: message must not be empty\n')
              return { exitCode: 1 }
            }
            this.deliverToParent(childId, description, message)
            await io.stdout(parsed.json ? `${JSON.stringify({ accepted: true })}\n` : 'sent\n')
            return { exitCode: 0 }
          },
        },
      ],
    }
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
        DEMI_PARENT_SESSION_ID: this.parentSession?.id() ?? '',
        DEMI_SUBAGENT_DEPTH: '1',
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
      ...(result !== undefined ? { result } : {}),
    }
  }
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
