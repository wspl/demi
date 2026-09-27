import { z } from 'zod'
import type { Block } from '@demicodes/protocol'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'

/**
 * The commands a transcript remembers: name, start, output, and the end when
 * a stored view saw one. A stored view is history, so none of these is
 * running: liveness comes only from the session's `shell_output` events,
 * which the server replays for the commands it still owns when the session
 * opens. A reloaded page and a fork therefore show only what actually runs.
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
    const name = (block.toolName === 'shell_exec' ? script(block.input) : undefined)
      ?? previous?.name
      ?? view.shellId
    commands.set(view.commandId, {
      id: view.commandId,
      name,
      phase: 'exited',
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

/** The script of the `shell_exec` call `toolUseId` among `blocks`, the latest when a model reused the id. */
export function callScript(blocks: readonly Block[], toolUseId: string): string | undefined {
  const call = blocks.findLast(
    (block) => block.type === 'tool_call' && block.toolUseId === toolUseId,
  )
  return call?.type === 'tool_call' && call.toolName === 'shell_exec'
    ? script(call.input)
    : undefined
}

/** A `shell_exec` call's script, from its input's JSON text. */
function script(input: string): string | undefined {
  const parsed = z.object({ script: z.string() }).safeParse(parseInput(input))
  return parsed.success ? parsed.data.script : undefined
}

function parseInput(input: string): unknown {
  try {
    return JSON.parse(input)
  } catch {
    // A streaming tool call can still have incomplete JSON.
    return null
  }
}
