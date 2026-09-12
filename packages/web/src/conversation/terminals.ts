import { z } from 'zod'
import type { Block } from '@demicodes/core'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'

const shellViewSchema = z.object({
  kind: z.literal('shell'),
  status: z.enum(['running', 'exited', 'aborted']),
  shellId: z.string(),
  commandId: z.string(),
  runningMs: z.number(),
  chunks: z.array(
    z.object({
      stream: z.enum(['stdout', 'stderr']),
      text: z.string(),
    }),
  ),
})

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
    const parsed = shellViewSchema.safeParse(block.view)
    if (!parsed.success) {
      continue
    }
    const view = parsed.data
    const previous = commands.get(view.commandId)
    let name = previous?.name ?? view.shellId
    if (block.toolName === 'shell_exec') {
      const input = z
        .object({ script: z.string() })
        .safeParse(parseInput(block.input))
      if (input.success) {
        name = input.data.script
      }
    }
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

function parseInput(input: string): unknown {
  try {
    return JSON.parse(input)
  } catch {
    // A streaming tool call can still have incomplete JSON.
    return null
  }
}
