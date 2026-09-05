import { toolRenderKind } from '@demicodes/web-ui/agent/tool-rendering'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'

const TOOL_BLOCK_NAME = {
  shell_exec: 'ToolShellBlock',
  shell_status: 'ToolShellStatusBlock',
  shell_write: 'ToolShellWriteBlock',
  shell_abort: 'ToolShellAbortBlock',
  yield: 'ToolYieldBlock',
  generic: 'ToolGenericBlock',
} as const

export function specimenForBlock(block: MessageListBlock): { name: string; variant: string } {
  switch (block.type) {
    case 'user':
      return { name: 'UserBlock', variant: 'user' }
    case 'steer':
      return { name: 'UserBlock', variant: 'steer' }
    case 'pending_steer':
      return { name: 'UserBlock', variant: 'pending' }
    case 'queue_divider':
      return { name: 'QueueDivider', variant: '' }
    case 'queued_message':
      return { name: 'UserBlock', variant: 'queued' }
    case 'thinking':
      return { name: 'ThinkingBlock', variant: block.id === 'thinking-streaming' ? 'streaming' : 'done' }
    case 'text':
      return { name: 'AssistantTextBlock', variant: '' }
    case 'tool_call':
      return {
        name: TOOL_BLOCK_NAME[toolRenderKind(block.toolName)],
        variant: `${block.toolName} · ${block.status}`,
      }
    case 'error':
      return { name: 'ErrorBlock', variant: block.code ?? '' }
    case 'abort':
      return { name: 'AbortedBlock', variant: '' }
    case 'compaction_boundary':
      return { name: 'CompactionBlock', variant: '' }
    default:
      return { name: block.type, variant: '' }
  }
}
