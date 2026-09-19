import type { Block } from '@demicodes/core'

/** A tool call block: @demicodes/core exports only the Block union. */
export type ToolCallBlock = Extract<Block, { type: 'tool_call' }>
