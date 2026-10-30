import type { Block, ToolView } from '@demicodes/protocol'

export type ToolCallBlock = Extract<Block, { type: 'tool_call' }>
/** A step its vendor ran on its side, drawn by its view. */
export type OpaqueBlock = Extract<Block, { type: 'opaque' }>
/**
 * An agent's move to another account after a usage limit
 * (`crates/provider-registry/src/rotation.rs`).
 */
export type AccountSwitchBlock = Extract<Block, { type: 'account_switch' }>

/** A shell tool's view of its command. */
export type ShellToolView = Extract<ToolView, { kind: 'shell' }>
