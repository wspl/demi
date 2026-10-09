import { z } from 'zod'
import type { Block } from '@demicodes/protocol'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { standardToolTitle } from '@demicodes/web-ui/agent/tool-rendering'

/**
 * The commands a transcript remembers: title, script, start, output, and the end when
 * a stored view saw one. A stored view is history, so none of these is
 * running: liveness comes only from the session's `shell_output` events,
 * which the server replays for the commands it still owns when the session
 * opens. A reloaded page and a fork therefore show only what actually runs,
 * and a command a stored view saw stopped stays stopped.
 */
export function transcriptTerminals(blocks: readonly Block[]): TerminalRecord[] {
  const commands = new Map<string, TerminalRecord>()
  for (const block of blocks) {
    if (block.type !== 'tool_call') {
      continue
    }
    const view = block.view
    if (view?.kind !== 'shell') {
      continue
    }
    const previous = commands.get(view.commandId)
    // The call that started it names it; a look at it does not.
    const call = (block.toolName === 'shell_exec' ? shellCall(block.input) : undefined) ?? previous
    commands.set(view.commandId, {
      id: view.commandId,
      title: call?.title ?? view.commandId,
      script: call?.script,
      phase: view.status === 'aborted' ? 'aborted' : 'exited',
      exitCode: view.status === 'exited' ? view.exitCode : undefined,
      startedAt: previous?.startedAt ?? block.createdAt,
      ...(view.status !== 'running'
        ? {
            endedAt: new Date(
              Date.parse(previous?.startedAt ?? block.createdAt) + view.runningMs,
            ).toISOString(),
          }
        : {}),
      output: view.chunks.map((chunk) => chunk.text).join(''),
    })
  }
  return [...commands.values()]
}

/** A `shell_exec` call's title, as its row shows it, and its script. */
export interface ShellCall {
  title: string
  script: string
}

/** The `shell_exec` call `toolUseId` among `blocks`, the latest when a model reused the id. */
export function findShellCall(blocks: readonly Block[], toolUseId: string): ShellCall | undefined {
  const call = blocks.findLast(
    (block) => block.type === 'tool_call' && block.toolUseId === toolUseId,
  )
  return call?.type === 'tool_call' && call.toolName === 'shell_exec'
    ? shellCall(call.input)
    : undefined
}

const shellCallInput = z.object({ script: z.string(), description: z.string().optional() })

/** A `shell_exec` call's title and script, from its input's JSON text. */
function shellCall(input: string): ShellCall | undefined {
  const parsed = shellCallInput.safeParse(parseInput(input))
  return parsed.success
    ? { title: standardToolTitle('shell_exec', parsed.data), script: parsed.data.script }
    : undefined
}

function parseInput(input: string): unknown {
  try {
    return JSON.parse(input)
  } catch {
    // A streaming tool call can still have incomplete JSON.
    return null
  }
}
