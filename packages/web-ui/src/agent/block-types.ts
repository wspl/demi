import type { Block, ToolView } from '@demicodes/protocol'

/** A tool call block. */
export type ToolCallBlock = Extract<Block, { type: 'tool_call' }>

/** A shell tool's view of its command. */
export type ShellToolView = Extract<ToolView, { kind: 'shell' }>
