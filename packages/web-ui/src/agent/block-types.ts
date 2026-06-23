import type { Block } from '@demi/core'

// agent-gui exposed per-variant block interfaces; demi @demi/core exposes only the Block
// union, so we recover the named variants here for the ported components.

export type UserBlock = Extract<Block, { type: 'user' }>
export type SteerBlock = Extract<Block, { type: 'steer' }>
export type TextBlock = Extract<Block, { type: 'text' }>
export type ThinkingBlock = Extract<Block, { type: 'thinking' }>
export type ToolCallBlock = Extract<Block, { type: 'tool_call' }>
export type ResponseBlock = Extract<Block, { type: 'response' }>
export type ErrorBlock = Extract<Block, { type: 'error' }>
