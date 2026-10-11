import { z } from 'zod'
import type { Block } from '@demicodes/protocol'
import { shellTitle } from '@demicodes/web-ui/agent/tool-rendering'

/** A `shell` call's title, as its row shows it, and its script, which a light block leaves out. */
export interface ShellCall {
  title: string
  script?: string
}

/** The call block `toolUseId` among `blocks`, the latest when a model reused the id. */
export function findCallBlock<B extends Block>(blocks: readonly B[], toolUseId: string): B | undefined {
  return blocks.findLast((block) => block.type === 'tool_call' && block.toolUseId === toolUseId)
}

/** The `shell` call `toolUseId` among `blocks`, the latest when a model reused the id. */
export function findShellCall(blocks: readonly Block[], toolUseId: string): ShellCall | undefined {
  const call = findCallBlock(blocks, toolUseId)
  return call ? shellCallOf(call) : undefined
}

/** A `shell` call block's title and script; nothing for another block. */
export function shellCallOf(block: Block): ShellCall | undefined {
  return block.type === 'tool_call' && block.toolName === 'shell' ? shellCall(block.input) : undefined
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
