import { truncate } from '@demi/utils'
import type { InferenceItem } from '@demi/provider'

/** Rough token estimate from character count (~4 chars/token). */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4)
}

/** The next (smaller) cut point to retry compaction with, or null when nothing more can be compacted. */
export function nextSmallerCompactionCutPoint(startIndex: number, cutPoint: number): number | null {
  const compactedBlockCount = cutPoint - startIndex
  if (compactedBlockCount <= 1) return null
  return startIndex + Math.max(1, Math.floor(compactedBlockCount / 2))
}

/** Renders normalized inference items into plain, delimited text for a compaction summary prompt. */
export function renderItemsForSummary(items: InferenceItem[]): string {
  const lines: string[] = []
  for (const item of items) {
    switch (item.type) {
      case 'user_message': {
        const text = item.content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join(' ')
        lines.push(`User: ${text}`)
        break
      }
      case 'user_steer': {
        const text = item.content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join(' ')
        lines.push(`User steer: ${text}`)
        break
      }
      case 'assistant_text':
        if (item.text.trim()) lines.push(`Assistant: ${item.text}`)
        break
      case 'tool_use':
        lines.push(`Assistant ran tool ${item.toolName}(${summaryShort(item.input)})`)
        break
      case 'tool_result': {
        const text = item.output.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join(' ')
        lines.push(`Tool result${item.isError ? ' (error)' : ''}: ${text}`)
        break
      }
      // assistant_thinking / assistant_redacted_thinking are intentionally omitted from summaries.
    }
  }
  return lines.join('\n')
}

/** A short, JSON-ish, length-capped rendering of an arbitrary value (for tool-input summaries). */
export function summaryShort(value: unknown): string {
  let text: string
  try {
    text = JSON.stringify(value) ?? String(value)
  } catch {
    text = String(value)
  }
  return truncate(text, 200)
}
