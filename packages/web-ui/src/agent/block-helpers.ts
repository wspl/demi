import type { Block, TokenUsage } from '@demi/core'

type ToolCallBlock = Extract<Block, { type: 'tool_call' }>

export interface ShellTerminalOutputChunk {
  stream: 'stdout' | 'stderr'
  text: string
}

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

export function shellTerminalOutputChunks(block: ToolCallBlock): ShellTerminalOutputChunk[] {
  const chunks = outputChunks(block.metadata)
  if (chunks.length > 0) return chunks

  const stdout = artifactDelta(block.metadata, 'stdout') || stdoutSection(toolOutputText(block))
  const stderr = artifactDelta(block.metadata, 'stderr') || stderrSection(toolOutputText(block))
  return [
    ...(stdout ? [{ stream: 'stdout' as const, text: stdout }] : []),
    ...(stderr ? [{ stream: 'stderr' as const, text: stderr }] : []),
  ]
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

function artifactDelta(metadata: unknown, name: 'stdout' | 'stderr'): string {
  if (!isRecord(metadata)) return ''
  const artifact = metadata[name]
  if (!isRecord(artifact)) return ''
  const delta = artifact['delta']
  if (typeof delta === 'string' && delta.length > 0) return delta
  const tail = artifact['tail']
  return typeof tail === 'string' ? tail : ''
}

function outputChunks(metadata: unknown): ShellTerminalOutputChunk[] {
  if (!isRecord(metadata)) return []
  const output = metadata['output']
  if (!isRecord(output) || !Array.isArray(output['chunks'])) return []
  return output['chunks'].flatMap((chunk): ShellTerminalOutputChunk[] => {
    if (!isRecord(chunk)) return []
    const stream = chunk['stream']
    const text = chunk['text']
    if ((stream !== 'stdout' && stream !== 'stderr') || typeof text !== 'string' || text.length === 0) return []
    return [{ stream, text }]
  })
}

function stdoutSection(text: string): string {
  return namedSection(text, 'stdout', ['stderr:'])
}

function stderrSection(text: string): string {
  return namedSection(text, 'stderr', ['next:'])
}

function namedSection(text: string, name: 'stdout' | 'stderr', nextNames: string[]): string {
  const header = `${name}:\n`
  const marker = `\n${header}`
  const start = text.startsWith(header) ? 0 : text.indexOf(marker)
  if (start < 0) return ''
  const bodyStart = start + (start === 0 ? header.length : marker.length)
  const lines = text.slice(bodyStart).split('\n')
  const section: string[] = []
  for (const line of lines) {
    if (
      line.startsWith(`${name}Path:`)
      || line.startsWith(`${name}Offset:`)
      || line.startsWith(`${name}Bytes:`)
      || line === `${name}: truncated`
      || nextNames.some((nextName) => line === nextName || line.startsWith(nextName))
    ) {
      break
    }
    section.push(line)
  }
  const result = section.join('\n').trimEnd()
  return result === '(empty)' ? '' : result
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}
