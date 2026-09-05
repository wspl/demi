// The `demi agent` command-tree definition. Pure declaration: every action
// goes through the generic `SubagentCommandOps` seam the supervisor provides,
// so this module knows nothing about job internals (the Job type parameter is
// opaque here) and the supervisor keeps its lifecycle methods private.
import { errorMessage } from '@demicodes/utils'
import { z } from 'zod'
import { isCommandGroup, type Command, type CommandGroup, type CommandIO } from '@demicodes/shell'
import { flattenTree, renderTreeNode, type AgentTreeNode } from './format'

const SPAWN_RUNNING_HINT =
  "next: the child agent is still working; a long-running spawn is normal. shell_write steers it, shell_abort aborts it. Otherwise stop attending and end the turn — the child's completion returns as this command's result and wakes the session when it is idle. Do not poll with shell_status or timed yields; use `demi agent show` only to decide a steer or abort."

const SPAWN_PROMPT_DESCRIPTION =
  "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return."

export interface AttendChildContext {
  io: CommandIO
  isJson: boolean
  signal: AbortSignal
  stdinStream: AsyncIterable<Uint8Array>
}

/** What the command tree needs from the supervisor; `Job` stays opaque here. */
export interface SubagentCommandOps<Job> {
  canSpawn: boolean
  profileNames(): string[]
  spawn(input: { prompt: string; profileName: string | undefined; description: string; isSpawnForbidden: boolean }): Promise<Job>
  resumeArchived(id: string, message: string): Promise<Job>
  attend(job: Job, ctx: AttendChildContext): Promise<{ exitCode: number }>
  getRunning(id: string): Job | null
  send(id: string, message: string): string
  steer(id: string, message: string): Promise<string>
  abortSubtree(id: string): Promise<void>
  tree(): Promise<AgentTreeNode[]>
  ownerId(): string
  show(id: string): { snapshot: Record<string, unknown>; text: string } | null
}

export function subagentCommandNode<Job>(ops: SubagentCommandOps<Job>): CommandGroup {
  const profileNames = ops.profileNames()
  const subcommands: Command[] = [
    {
      name: 'spawn',
      kind: 'rpc',
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
          .describe(`Named subagent profile configured at harness assembly; omit to inherit the parent's model, prompt, Host and commands. Available: ${profileNames.length > 0 ? profileNames.join(', ') : 'none'}.`),
        description: z
          .string()
          .optional()
          .describe('Short UI title distinguishing concurrent children.'),
        'no-subagents': z.boolean().optional().describe('Forbid this child from spawning subagents of its own; it can still send, steer, list, and show.'),
      },
      positionals: ['prompt'],
      stdinField: 'prompt',
      output: { json: z.object({ subagentId: z.string(), text: z.string() }) },
      runningHint: SPAWN_RUNNING_HINT,
      run: async ({ parsed, io, signal, stdinStream }) => {
        const prompt = String(parsed.values.prompt ?? '').trim()
        if (!prompt) {
          await io.stderr('demi agent spawn: prompt must not be empty\n')
          return { exitCode: 1 }
        }
        let job: Job
        try {
          job = await ops.spawn({
            prompt,
            profileName: parsed.values.profile === undefined ? undefined : String(parsed.values.profile),
            description: parsed.values.description === undefined ? '' : String(parsed.values.description),
            isSpawnForbidden: parsed.values['no-subagents'] === true,
          })
        } catch (error) {
          await io.stderr(`demi agent spawn: ${errorMessage(error)}\n`)
          return { exitCode: 1 }
        }
        return ops.attend(job, { io, isJson: parsed.json === true, signal, stdinStream })
      },
    },
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
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const message = String(parsed.values.message ?? '').trim()
        if (!message) {
          await io.stderr('demi agent send: message must not be empty\n')
          return { exitCode: 1 }
        }
        try {
          const targetId = ops.send(String(parsed.values.id), message)
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
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const message = String(parsed.values.message ?? '').trim()
        if (!message) {
          await io.stderr('demi agent steer: message must not be empty\n')
          return { exitCode: 1 }
        }
        try {
          const targetId = await ops.steer(String(parsed.values.id), message)
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
      input: { id: z.string().describe('subagentId from spawn stderr') },
      positionals: ['id'],
      output: { json: z.object({ id: z.string(), aborted: z.boolean() }) },
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const id = String(parsed.values.id)
        if (!ops.getRunning(id)) {
          await io.stderr(`demi agent abort: "${id}" is not one of your running children\n`)
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
        'Revive one of your own archived (finished) children with a new user message on top of its preserved transcript. Behaves like the spawn command afterwards: stays running until the child ends again, stdout is its new last assistant text, shell_write steers, shell_abort aborts. Archived ids are in `demi agent list`.',
      input: {
        id: z.string().describe('subagentId of an archived child'),
        message: z.string().optional().describe('The reviving user message; positional, or stdin/heredoc when omitted.'),
      },
      positionals: ['id', 'message'],
      stdinField: 'message',
      output: { json: z.object({ subagentId: z.string(), text: z.string() }) },
      runningHint: SPAWN_RUNNING_HINT,
      kind: 'rpc',
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
        'Render the whole session tree from the root down, marking your own position. Live agents show phase, ages, execution, and activity; each node\'s archived (finished, revivable by its parent) children render beneath it. Every age is relative to now. A read, not a wait — not for polling loops.',
      output: { json: z.object({ tree: z.array(z.unknown()) }) },
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const nodes = await ops.tree()
        if (parsed.json) {
          await io.stdout(`${JSON.stringify({ tree: flattenTree(nodes, ops.ownerId()) })}\n`)
          return { exitCode: 0 }
        }
        const lines: string[] = []
        for (const node of nodes) renderTreeNode(node, '', true, ops.ownerId(), lines)
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
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const id = String(parsed.values.id)
        const entry = ops.show(id)
        if (!entry) {
          await io.stderr(`demi agent show: no live agent "${id}"\n`)
          return { exitCode: 1 }
        }
        if (parsed.json) await io.stdout(`${JSON.stringify({ agent: entry.snapshot })}\n`)
        else await io.stdout(entry.text)
        return { exitCode: 0 }
      },
    },
  ]
  return {
    name: 'agent',
    summary: ops.canSpawn
      ? 'Agent tree: spawn and manage your own children; send, steer, list and show any live agent.'
      : 'Agent tree communication: this session may not spawn subagents; send, steer, list and show any live agent.',
    subcommands: subcommands.filter((command) => ops.canSpawn || !['spawn', 'abort', 'resume'].includes(command.name)),
  }
}

/**
 * The `agent` node's shape alone, for a manifest built outside any session:
 * a target parses `demi agent …` from it and relays the call as `rpc`, which
 * runs against the live session's tree. Running the shape itself is a wiring
 * error.
 */
export function subagentCommandShape(profileNames: string[]): CommandGroup {
  const notHere = (): never => {
    throw new Error('demi agent runs on the live session; the manifest carries its shape only')
  }
  return subagentCommandNode<never>({
    canSpawn: true,
    profileNames: () => profileNames,
    spawn: notHere,
    resumeArchived: notHere,
    attend: notHere,
    getRunning: notHere,
    send: notHere,
    steer: notHere,
    abortSubtree: notHere,
    tree: notHere,
    ownerId: notHere,
    show: notHere,
  })
}

/**
 * Grafts the `agent` node under a `demi` root: onto an existing harness `demi`
 * tree, or as a new `demi` root when the harness has none.
 */
export function injectSubagentCommand(commands: Command[], agentNode: CommandGroup): Command[] {
  const demiIndex = commands.findIndex((command) => command.name === 'demi')
  if (demiIndex === -1) {
    return [...commands, { name: 'demi', summary: 'Demi agent runtime commands.', subcommands: [agentNode] }]
  }
  const demi = commands[demiIndex]!
  if (!isCommandGroup(demi)) throw new Error('injectSubagentCommand: the demi root must be a group')
  const subcommands = [...demi.subcommands.filter((command) => command.name !== 'agent'), agentNode]
  const next = [...commands]
  next[demiIndex] = { ...demi, subcommands }
  return next
}
