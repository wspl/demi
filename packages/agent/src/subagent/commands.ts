// The `demi agent` command-tree definition. Pure declaration: every action
// goes through `SubagentCommandOps`; the supervisor owns child lifecycles.
import { createId, errorMessage } from '@demicodes/utils'
import { z } from 'zod'
import {
  isCommandGroup,
  type Command,
  type CommandGroup,
  type CommandStorage
} from '@demicodes/shell'
import { flattenTree, renderTreeNode, type AgentTreeNode } from './format'

const requestIdSchema = z.string().min(1).max(128).optional().describe(
  'Stable id for this creation or resume request. Supply the same id and arguments to retry safely after an uncertain response; otherwise a new id is generated.'
)

const SPAWN_PROMPT_DESCRIPTION =
  "The child's first user message and only task brief. The child starts with an empty transcript and cannot see this conversation: do not refer to prior turns, and do not paste this conversation or the product user's message unchanged. Include the goal for this child, applicable decisions and constraints, whether to edit or only report, how to verify, and every concrete identifier it needs (paths, ids, error text, commands already tried and their key results). State the exact shape of the last assistant text it should return."

/** The lifecycle, communication, and read operations the command tree needs. */
export interface SubagentCommandOps {
  canSpawn: boolean
  profileNames(): string[]
  spawn(input: {
    prompt: string;
    profileName: string | undefined;
    description: string;
    isSpawnForbidden: boolean
  }, requestId: string, storage: CommandStorage): Promise<string>
  resumeArchived(id: string, message: string, requestId: string, storage: CommandStorage): Promise<string>
  getRunning(id: string): unknown | null
  send(id: string, message: string): Promise<string>
  abortSubtree(id: string): Promise<void>
  tree(): Promise<AgentTreeNode[]>
  ownerId(): string
  show(
    id: string
  ): {
    snapshot: Record<string, unknown>;
    text: string
  } | null
}

export function subagentCommandNode(
  ops: SubagentCommandOps
): CommandGroup {
  const profileNames = ops.profileNames()
  const subcommands: Command[] = [
    {
      name: 'spawn',
      kind: 'rpc',
      summary:
        'Start a child agent session and return its id immediately after creation. The child runs independently of this command. Completion arrives as a message to the parent, waking it when idle. When you have no independent work left, end your turn and let the completion message wake you. Do not poll agent list/show or schedule timed yield calls to wait for children. Use agent send to communicate and agent abort to stop it. Children can spawn children of their own.',
      successOutput:
        'stdout is "subagentId: <id>"; creation succeeded, not necessarily execution',
      failureOutput: 'non-zero exit with the creation failure reason on stderr',
      input: {
        prompt: z.string().describe(SPAWN_PROMPT_DESCRIPTION),
        'request-id': requestIdSchema,
        profile: z
          .string()
          .optional()
          .describe(`Named subagent profile configured at harness assembly; omit to inherit the parent's model, prompt, Host and commands. Available: ${profileNames.length > 0 ? profileNames.join(', ') : 'none'}.`),
        description: z
          .string()
          .optional()
          .describe('Short UI title distinguishing concurrent children.'),
        'no-subagents': z.boolean().optional().describe(
          'Forbid this child from spawning subagents of its own; it can still send, list, and show.'
        ),
      },
      stdinField: 'prompt',
      output: { json: z.object({ subagentId: z.string() }) },
      run: async ({ parsed, io, storage }) => {
        const prompt = (parsed.values.prompt as string).trim()
        if (!prompt) {
          await io.stderr('demi agent spawn: prompt must not be empty\n')
          return { exitCode: 1 }
        }
        let subagentId: string
        try {
          subagentId = await ops.spawn({
            prompt,
            profileName: parsed.values.profile === undefined
              ? undefined
              : String(parsed.values.profile),
            description: parsed.values.description === undefined
              ? ''
              : String(parsed.values.description),
            isSpawnForbidden: parsed.values['no-subagents'] === true,
          }, String(parsed.values['request-id'] ?? createId()), storage)
        } catch (error) {
          await io.stderr(`demi agent spawn: ${errorMessage(error)}\n`)
          return { exitCode: 1 }
        }
        await io.stdout(parsed.json
          ? `${JSON.stringify({ subagentId })}\n`
          : `subagentId: ${subagentId}\n`)
        return { exitCode: 0 }
      },
    },
    {
      name: 'send',
      summary:
        'Deliver information to any live agent in the tree, or parent. A busy recipient incorporates it through internal steering; an idle recipient wakes. Returns after durable acceptance, without waiting for an answer. Use for interim information, questions, or blockers; your final answer is delivered automatically. Archived recipients must be reopened by their parent with resume.',
      input: {
        id: z.string().describe(
          'Target agent id from the tree, or "parent" for the session that spawned this one'
        ),
        message: z.string()
          .describe('Message body.'),
      },
      positionals: ['id'],
      stdinField: 'message',
      output: { json: z.object({ id: z.string(), accepted: z.boolean() }) },
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const message = (parsed.values.message as string).trim()
        if (!message) {
          await io.stderr('demi agent send: message must not be empty\n')
          return { exitCode: 1 }
        }
        try {
          const targetId = await ops.send(String(parsed.values.id), message)
          await io.stdout(parsed.json
            ? `${JSON.stringify({ id: targetId, accepted: true })}\n`
            : `sent to ${targetId}\n`)
          return { exitCode: 0 }
        } catch (error) {
          await io.stderr(`demi agent send: ${errorMessage(error)}\n`)
          return { exitCode: 1 }
        }
      },
    },
    {
      name: 'abort',
      summary: 'Abort one of your own running children and its whole subtree. Siblings are untouched; only the spawning session may abort a child.',
      input: { id: z.string().describe('subagentId from spawn stdout') },
      positionals: ['id'],
      output: { json: z.object({ id: z.string(), aborted: z.boolean() }) },
      kind: 'rpc',
      run: async ({ parsed, io }) => {
        const id = String(parsed.values.id)
        if (!ops.getRunning(id)) {
          await io.stderr(
            `demi agent abort: "${id}" is not one of your running children\n`
          )
          return { exitCode: 1 }
        }
        await ops.abortSubtree(id)
        await io.stdout(parsed.json
          ? `${JSON.stringify({ id, aborted: true })}\n`
          : `aborted ${id}\n`)
        return { exitCode: 0 }
      },
    },
    {
      name: 'resume',
      summary:
        'Revive one of your own archived children with a new user message on its preserved transcript. Return its id immediately after accepting the message; completion is delivered separately to the parent. Use agent send to communicate and agent abort to stop it. Archived ids are in agent list.',
      input: {
        id: z.string().describe('subagentId of an archived child'),
        'request-id': requestIdSchema,
        message: z.string().describe(
          'The reviving user message.'
        ),
      },
      positionals: ['id'],
      stdinField: 'message',
      output: { json: z.object({ subagentId: z.string() }) },
      kind: 'rpc',
      run: async ({ parsed, io, storage }) => {
        const id = String(parsed.values.id)
        const message = (parsed.values.message as string).trim()
        if (!message) {
          await io.stderr('demi agent resume: message must not be empty\n')
          return { exitCode: 1 }
        }
        let subagentId: string
        try {
          subagentId = await ops.resumeArchived(id, message, String(parsed.values['request-id'] ?? createId()), storage)
        } catch (error) {
          await io.stderr(`demi agent resume: ${errorMessage(error)}\n`)
          return { exitCode: 1 }
        }
        await io.stdout(parsed.json
          ? `${JSON.stringify({ subagentId })}\n`
          : `subagentId: ${subagentId}\n`)
        return { exitCode: 0 }
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
          await io.stdout(
            `${JSON.stringify({ tree: flattenTree(nodes, ops.ownerId()) })}\n`
          )
          return { exitCode: 0 }
        }
        const lines: string[] = []
        for (const node of nodes) renderTreeNode(
          node,
          '',
          true,
          ops.ownerId(),
          lines
        )
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
      ? 'Agent tree: spawn and manage your own children; send, list and show any live agent.'
      : 'Agent tree communication: this session may not spawn subagents; send, list and show any live agent.',
    subcommands: subcommands.filter((command) => ops.canSpawn || ![
      'spawn',
      'abort',
      'resume'
    ].includes(command.name)),
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
    throw new Error(
      'demi agent runs on the live session; the manifest carries its shape only'
    )
  }
  return subagentCommandNode({
    canSpawn: true,
    profileNames: () => profileNames,
    spawn: notHere,
    resumeArchived: notHere,
    getRunning: notHere,
    send: notHere,
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
export function injectSubagentCommand(
  commands: Command[],
  agentNode: CommandGroup
): Command[] {
  const demiIndex = commands.findIndex((command) => command.name === 'demi')
  if (demiIndex === -1) {
    return [
      ...commands,
      {
        name: 'demi',
        summary: 'Demi agent runtime commands.',
        subcommands: [agentNode]
      }
    ]
  }
  const demi = commands[demiIndex]!
  if (!isCommandGroup(demi))
    throw new Error('injectSubagentCommand: the demi root must be a group')
  const subcommands = [
    ...demi.subcommands.filter((command) => command.name !== 'agent'),
    agentNode
  ]
  const next = [...commands]
  next[demiIndex] = { ...demi, subcommands }
  return next
}
