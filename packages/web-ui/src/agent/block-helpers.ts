import type { Block, TokenUsage } from '@demi/core'

type ToolCallBlock = Extract<Block, { type: 'tool_call' }>

export function getLatestResponseUsage(blocks: readonly Block[]): TokenUsage | null {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const block = blocks[i]!
    if (block.type === 'response') return block.usage
  }
  return null
}

export function getToolErrorText(block: ToolCallBlock): string | undefined {
  if (block.status !== 'error') return undefined
  const texts: string[] = []
  for (const part of block.output) {
    if (part.type === 'text') texts.push(part.text)
  }
  return texts.length > 0 ? texts.join('\n') : undefined
}

export function toolOutputText(block: ToolCallBlock): string {
  const source = block.status === 'executing' ? block.streamingOutput : block.streamingOutput.length > 0 ? block.streamingOutput : block.output
  return source
    .filter((part): part is Extract<typeof part, { type: 'text' }> => part.type === 'text')
    .map((part) => part.text)
    .join('\n')
}

export function shellStderrText(block: ToolCallBlock): string {
  const fromMetadata = artifactDelta(block.metadata, 'stderr')
  if (fromMetadata) return fromMetadata
  return stderrSection(toolOutputText(block))
}

export function parseToolInput(raw: string): Record<string, unknown> {
  if (!raw) return {}
  try {
    const parsed = JSON.parse(raw)
    return typeof parsed === 'object' && parsed !== null ? (parsed as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}

function artifactDelta(metadata: unknown, name: 'stderr'): string {
  if (!isRecord(metadata)) return ''
  const artifact = metadata[name]
  if (!isRecord(artifact)) return ''
  const delta = artifact['delta']
  if (typeof delta === 'string' && delta.length > 0) return delta
  const tail = artifact['tail']
  return typeof tail === 'string' ? tail : ''
}

function stderrSection(text: string): string {
  const marker = '\nstderr:\n'
  const start = text.startsWith('stderr:\n') ? 0 : text.indexOf(marker)
  if (start < 0) return ''
  const bodyStart = start + (start === 0 ? 'stderr:\n'.length : marker.length)
  const lines = text.slice(bodyStart).split('\n')
  const stderr: string[] = []
  for (const line of lines) {
    if (
      line.startsWith('stderrPath:')
      || line.startsWith('stderrOffset:')
      || line.startsWith('stderrBytes:')
      || line === 'stderr: truncated'
      || line.startsWith('next:')
    ) {
      break
    }
    stderr.push(line)
  }
  const result = stderr.join('\n').trimEnd()
  return result === '(empty)' ? '' : result
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}
