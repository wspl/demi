import { z } from 'zod'
import type { Block } from '@demicodes/protocol'
import { shellTitle } from '@demicodes/web-ui/agent/tool-rendering'

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
