import type { Block, ToolView } from '@demicodes/protocol'

/** A tool call block. */
export type ToolCallBlock = Extract<Block, { type: 'tool_call' }>
/** An agent's move to another account after a usage limit (`crates/backend-providers/src/llm/rotation.rs`). */
export type AccountSwitchBlock = Extract<Block, { type: 'account_switch' }>

/** A shell tool's view of its command. */
export type ShellToolView = Extract<ToolView, { kind: 'shell' }>
