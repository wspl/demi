// The `demi agent` command-tree definition. Pure declaration: every action
// goes through the generic `SubagentCommandOps` seam the supervisor provides,
// so this module knows nothing about job internals (the Job type parameter is
// opaque here) and the supervisor keeps its lifecycle methods private.
import { errorMessage } from '@demicodes/utils'
import { z } from 'zod'
import type { Command, CommandIO } from '@demicodes/shell'
import { formatDuration } from './format'

const SPAWN_PROMPT_DESCRIPTION =
  "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return."

export interface AttendChildContext {
  io: CommandIO
  isJson: boolean
  signal: AbortSignal
  stdinStream: AsyncIterable<Uint8Array>
}

export interface ArchivedSubagentEntry {
  id: string
  description: string
  profileName: string | null
  closedPhase: string | undefined
  closedAt: number | undefined
}

/** What the command tree needs from the supervisor; `Job` stays opaque here. */
export interface SubagentCommandOps<Job> {
  profileNames(): string[]
  spawn(input: { prompt: string; profileName: string | undefined; description: string }): Promise<Job>
  resumeArchived(id: string, message: string): Promise<Job>
  attend(job: Job, ctx: AttendChildContext): Promise<{ exitCode: number }>
  getRunning(id: string): Job | null
  steer(job: Job, message: string): Promise<void>
  abortSubtree(id: string): Promise<void>
  runningJobs(): Job[]
  listArchived(): Promise<ArchivedSubagentEntry[]>
  snapshot(job: Job, detailed: boolean): Record<string, unknown>
  renderListLine(job: Job): string
  renderShow(job: Job): string
}

export function subagentCommandNode<Job>(ops: SubagentCommandOps<Job>): Command {
  const profileNames = ops.profileNames()
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
      let job: Job
      try {
        job = await ops.spawn({
          prompt,
          profileName: parsed.values.profile === undefined ? undefined : String(parsed.values.profile),
          description: parsed.values.description === undefined ? '' : String(parsed.values.description),
        })
      } catch (error) {
        await io.stderr(`demi agent: ${errorMessage(error)}\n`)
        return { exitCode: 1 }
      }
      return ops.attend(job, { io, isJson: parsed.json === true, signal, stdinStream })
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
          const job = ops.getRunning(id)
          if (!job) {
            await io.stderr(`demi agent steer: no running subagent "${id}"\n`)
            return { exitCode: 1 }
          }
          await ops.steer(job, message)
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
          if (!ops.getRunning(id)) {
            await io.stderr(`demi agent abort: no running subagent "${id}"\n`)
            return { exitCode: 1 }
          }
          await ops.abortSubtree(id)
          await io.stdout(parsed.json ? `${JSON.stringify({ id, aborted: true })}\n` : `aborted ${id}\n`)
          return { exitCode: 0 }
        },
      },
      {
        name: 'resume',
        summary:
          'Revive an archived (finished) child with a new user message on top of its preserved transcript. Behaves like the spawn command afterwards: stays running until the child ends again, stdout is its new last assistant text, shell_write steers, shell_abort aborts. Archived ids are in `demi agent list`.',
        input: {
          id: z.string().describe('subagentId of an archived child'),
          message: z.string().optional().describe('The reviving user message; positional, or stdin/heredoc when omitted.'),
        },
        positionals: ['id', 'message'],
        stdinField: 'message',
        output: { json: z.object({ subagentId: z.string(), text: z.string() }) },
        run: async ({ parsed, io, signal, stdinStream }) => {
          const id = String(parsed.values.id)
          const message = String(parsed.values.message ?? '').trim()
          if (!message) {
            await io.stderr('demi agent resume: message must not be empty\n')
            return { exitCode: 1 }
          }
          let job: Job
          try {
            job = await ops.resumeArchived(id, message)
          } catch (error) {
            await io.stderr(`demi agent resume: ${errorMessage(error)}\n`)
            return { exitCode: 1 }
          }
          return ops.attend(job, { io, isJson: parsed.json === true, signal, stdinStream })
        },
      },
      {
        name: 'list',
        summary:
          'Snapshot roster of this session\'s running children, then its archived (finished, revivable via resume) children. One line per child with ages relative to now. A read, not a wait — not for polling loops.',
        output: { json: z.object({ agents: z.array(z.unknown()), archived: z.array(z.unknown()) }) },
        run: async ({ parsed, io }) => {
          const jobs = ops.runningJobs()
          const archived = await ops.listArchived()
          if (parsed.json) {
            await io.stdout(`${JSON.stringify({
              agents: jobs.map((job) => ops.snapshot(job, false)),
              archived: archived.map((entry) => ({
                subagentId: entry.id,
                description: entry.description,
                profile: entry.profileName,
                phase: entry.closedPhase,
                closedAgoMs: entry.closedAt === undefined ? null : Date.now() - entry.closedAt,
              })),
            })}\n`)
            return { exitCode: 0 }
          }
          if (jobs.length === 0) await io.stdout('no running subagents\n')
          for (const job of jobs) await io.stdout(`${ops.renderListLine(job)}\n`)
          if (archived.length > 0) {
            await io.stdout('archived (revivable with `demi agent resume <id>`):\n')
            for (const entry of archived) {
              const closedAgo = entry.closedAt === undefined ? '' : `  closed ${formatDuration(Date.now() - entry.closedAt)} ago`
              await io.stdout(`  ${entry.id}  ${entry.closedPhase}${closedAgo}  ${entry.description ? `"${entry.description}"` : '(no description)'}\n`)
            }
          }
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
          const job = ops.getRunning(id)
          if (!job) {
            await io.stderr(`demi agent show: no running subagent "${id}"\n`)
            return { exitCode: 1 }
          }
          if (parsed.json) await io.stdout(`${JSON.stringify({ agent: ops.snapshot(job, true) })}\n`)
          else await io.stdout(ops.renderShow(job))
          return { exitCode: 0 }
        },
      },
    ],
  }
}

/** The child-side bridge node: `demi agent send-parent` inside a child session. */
export function childAgentNode(deliverToParent: (message: string) => void): Command {
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
          deliverToParent(message)
          await io.stdout(parsed.json ? `${JSON.stringify({ accepted: true })}\n` : 'sent\n')
          return { exitCode: 0 }
        },
      },
    ],
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
