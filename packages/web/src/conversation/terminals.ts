import { z } from 'zod'
import type { Block } from '@demicodes/protocol'
import type { CommandRecord } from '../api/generated/web-api'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { shellTitle } from '@demicodes/web-ui/agent/tool-rendering'

/**
 * The terminal of an ended command, from its record (`web-api.md`
 * § Subagents and commands): title, script, start, output, and its end as
 * the stored view saw it. A stored view is history, so it is never running:
 * liveness comes only from the session's `shell_output` events, and a
 * command a stored view saw stopped stays stopped.
 */
export function commandTerminal(record: CommandRecord): TerminalRecord {
  const view = record.view?.kind === 'shell' ? record.view : null
  return {
    id: record.commandId,
    title: record.title || record.commandId,
    script: record.script || undefined,
    phase: view?.status === 'aborted' ? 'aborted' : 'exited',
    exitCode: view?.status === 'exited' ? view.exitCode : undefined,
    startedAt: record.startedAt,
    ...(view && view.status !== 'running'
      ? { endedAt: new Date(Date.parse(record.startedAt) + view.runningMs).toISOString() }
      : {}),
    output: view?.chunks.map((chunk) => chunk.text).join('') ?? '',
    ...(record.subagentId === null ? {} : { subagentId: record.subagentId }),
  }
}

/** A `shell` call's title, as its row shows it, and its script, which a light block leaves out. */
export interface ShellCall {
  title: string
  script?: string
}

/** The `shell` call `toolUseId` among `blocks`, the latest when a model reused the id. */
export function findShellCall(blocks: readonly Block[], toolUseId: string): ShellCall | undefined {
  const call = blocks.findLast(
    (block) => block.type === 'tool_call' && block.toolUseId === toolUseId,
  )
  return call?.type === 'tool_call' && call.toolName === 'shell'
    ? shellCall(call.input)
    : undefined
}

const shellCallInput = z.object({ script: z.string().optional(), description: z.string().optional() })

/** A `shell` call's title and script, from its input's JSON text. */
function shellCall(input: string): ShellCall | undefined {
  const parsed = shellCallInput.safeParse(parseInput(input))
  return parsed.success
    ? { title: shellTitle(parsed.data), ...(parsed.data.script === undefined ? {} : { script: parsed.data.script }) }
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
