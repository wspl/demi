import process from 'node:process'
import type { Block, SessionPhase, TokenUsage, UserContentBlock } from '@demicodes/core'
import type { ClientSessionEvent } from '@demicodes/agent'
import { color, writeEventLine, writeLineTo, writePrefixed, type ReplOutput, type Tone } from './output'

export interface RenderState {
  output: ReplOutput
  phase: SessionPhase | null
  textLengths: Map<string, number>
  thinkingLengths: Map<string, number>
  seenThinkingSignatures: Set<string>
  toolStatuses: Map<string, string>
  seenResponseIds: Set<string>
  seenUserIds: Set<string>
  seenSteerIds: Set<string>
  seenErrorIds: Set<string>
  seenAbortIds: Set<string>
  toolOutputCounts: Map<string, number>
  activeStream: 'assistant' | 'thinking' | null
  streamAtLineStart: boolean
}

export interface ReplEventSource {
  subscribe(listener: (event: ClientSessionEvent) => void): () => void
}

export function createRenderer(output: ReplOutput = process.stdout): RenderState {
  return {
    output,
    phase: null,
    textLengths: new Map(),
    thinkingLengths: new Map(),
    seenThinkingSignatures: new Set(),
    toolStatuses: new Map(),
    seenResponseIds: new Set(),
    seenUserIds: new Set(),
    seenSteerIds: new Set(),
    seenErrorIds: new Set(),
    seenAbortIds: new Set(),
    toolOutputCounts: new Map(),
    activeStream: null,
    streamAtLineStart: true,
  }
}

export function attachRenderer(source: ReplEventSource, state: RenderState): () => void {
  return source.subscribe((event) => renderEvent(state, event))
}

export function renderEvent(state: RenderState, event: ClientSessionEvent): void {
  switch (event.type) {
    case 'opened':
      return
    case 'phase':
      if (event.phase !== state.phase) {
        finishStream(state)
        state.phase = event.phase
        writeEventLine(state.output, 'state', event.phase, event.phase === 'idle' ? 'green' : 'yellow')
      }
      return
    case 'queue':
      finishStream(state)
      if (event.queue.length > 0) {
        writeEventLine(state.output, 'queue', `${event.queue.length} pending`, 'dim')
      }
      return
    case 'shell_output':
      finishStream(state)
      renderShellOutput(state, event.commandId, event.snapshot.stdout.delta, event.snapshot.stderr.delta)
      return
    case 'audit':
      finishStream(state)
      for (const item of event.events) {
        if (item.kind === 'registered-command') {
          writeEventLine(state.output, 'audit', `registered ${item.name} ${item.args.join(' ')} -> ${item.exitCode}`, 'dim')
        } else {
          writeEventLine(
            state.output,
            'audit',
            `system ${item.name} ${item.args.join(' ')} -> ${item.exitCode ?? 'signal'}`,
            'dim',
          )
        }
      }
      return
    case 'tool_progress':
      renderToolProgress(state, event.output)
      return
    case 'retry_scheduled':
      finishStream(state)
      writeEventLine(
        state.output,
        'state',
        `provider ${event.code ?? 'transient error'}; retry #${event.attempt} in ${event.delayMs}ms`,
        'yellow',
      )
      return
    case 'shell_write_result':
      finishStream(state)
      return
    case 'abort_result':
      finishStream(state)
      writeEventLine(
        state.output,
        'state',
        event.result.aborted
          ? `aborted ${event.result.target}${event.result.canAbortAgain ? '; more abortable work remains' : ''}`
          : 'nothing to abort',
        event.result.aborted ? 'yellow' : 'dim',
      )
      return
    case 'error':
      finishStream(state)
      writeEventLine(state.output, 'error', event.message, 'red')
      return
    case 'rejected':
      finishStream(state)
      writeEventLine(state.output, 'error', `${event.command} rejected: ${event.reason}`, 'red')
      return
    case 'closed':
      finishStream(state)
      writeEventLine(state.output, 'state', 'closed', 'dim')
      return
    case 'transcript_snapshot':
      renderBlocks(state, event.blocks)
      return
    case 'transcript_patch':
      renderBlocks(state, event.blocks)
      return
  }
}

function renderBlocks(state: RenderState, blocks: Block[]): void {
  for (const block of blocks) {
    // Hidden user/steer turns (internal yield wakeups) drive the model but are not shown.
    if ((block.type === 'user' || block.type === 'steer') && block.hidden) continue
    if (block.type === 'user') {
      if (!state.seenUserIds.has(block.id)) state.seenUserIds.add(block.id)
      continue
    }
    if (block.type === 'steer') {
      if (!state.seenSteerIds.has(block.id)) {
        finishStream(state)
        state.seenSteerIds.add(block.id)
        writeEventLine(state.output, 'steer', formatUserContent(block.content), 'yellow')
      }
      continue
    }
    if (block.type === 'text') {
      const previous = state.textLengths.get(block.id) ?? 0
      const delta = block.text.slice(previous)
      if (delta) writeStreamDelta(state, 'assistant', 'assistant> ', 'blue', delta)
      state.textLengths.set(block.id, block.text.length)
      continue
    }
    if (block.type === 'thinking') {
      const previous = state.thinkingLengths.get(block.id) ?? 0
      const delta = block.text.slice(previous)
      if (delta) writeStreamDelta(state, 'thinking', 'thinking> ', 'dim', delta)
      state.thinkingLengths.set(block.id, block.text.length)
      if (block.signature && !state.seenThinkingSignatures.has(block.id)) {
        state.seenThinkingSignatures.add(block.id)
        if (block.text.length === 0) {
          finishStream(state)
          writeLineTo(state.output, color('thinking> [signed]', 'dim', state.output))
        }
      }
      continue
    }
    if (block.type === 'redacted_thinking') {
      if (!state.thinkingLengths.has(block.id)) {
        finishStream(state)
        writeLineTo(state.output, color(`thinking> [redacted ${block.data.length} chars]`, 'dim', state.output))
        state.thinkingLengths.set(block.id, block.data.length)
      }
      continue
    }
    if (block.type === 'tool_call') {
      const marker = `${block.status}:${block.output.length}:${block.streamingOutput.length}`
      if (state.toolStatuses.get(block.id) !== marker) {
        finishStream(state)
        state.toolStatuses.set(block.id, marker)
        const input = formatToolInput(block)
        writeEventLine(state.output, 'tool', `${block.toolName} ${block.status}${input ? ` -- ${input}` : ''}`, 'cyan')
      }
      renderToolCallOutput(state, block)
      continue
    }
    if (block.type === 'response' && !state.seenResponseIds.has(block.id)) {
      finishStream(state)
      state.seenResponseIds.add(block.id)
      writeEventLine(state.output, 'usage', formatUsage(block.usage), 'dim')
      continue
    }
    if (block.type === 'error' && !state.seenErrorIds.has(block.id)) {
      finishStream(state)
      state.seenErrorIds.add(block.id)
      writeEventLine(state.output, 'error', `agent ${block.message}`, 'red')
      continue
    }
    if (block.type === 'abort' && !state.seenAbortIds.has(block.id)) {
      finishStream(state)
      state.seenAbortIds.add(block.id)
      writeEventLine(state.output, 'state', 'turn aborted', 'yellow')
      continue
    }
  }
}

function renderToolCallOutput(state: RenderState, block: Extract<Block, { type: 'tool_call' }>): void {
  if (block.status !== 'error') return
  const previous = state.toolOutputCounts.get(block.id) ?? 0
  const next = block.output.slice(previous)
  if (next.length === 0) return
  finishStream(state)
  state.toolOutputCounts.set(block.id, block.output.length)
  const text = next.map((item) => (item.type === 'text' ? item.text : `[image:${item.source.mediaType}]`)).join('\n')
  writePrefixed(state.output, 'tool-error', text, 'red')
}

function renderShellOutput(state: RenderState, commandId: string, stdoutDelta: string, stderrDelta: string): void {
  if (stdoutDelta) writePrefixed(state.output, `shell[${commandId}] stdout`, stdoutDelta, 'green')
  if (stderrDelta) writePrefixed(state.output, `shell[${commandId}] stderr`, stderrDelta, 'red')
}

function renderToolProgress(state: RenderState, output: Extract<ClientSessionEvent, { type: 'tool_progress' }>['output']): void {
  const text = output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
  const shell = parseShellProgress(text)
  if (shell) {
    finishStream(state)
    writeEventLine(state.output, 'progress', `shell[${shell.shellId}] ${shell.status}${shell.reason ? ` (${shell.reason})` : ''}`, 'dim')
    return
  }
  if (text.trim()) {
    finishStream(state)
    writePrefixed(state.output, 'progress', text, 'dim')
  }
}

function parseShellProgress(text: string): { shellId: string; status: string; reason?: string } | null {
  try {
    const value = JSON.parse(text) as Record<string, unknown>
    const id = typeof value.commandId === 'string' ? value.commandId : value.shellId
    if (typeof id !== 'string' || typeof value.status !== 'string') return null
    return {
      shellId: id,
      status: value.status,
      reason: typeof value.reason === 'string' ? value.reason : undefined,
    }
  } catch {
    return null
  }
}

function writeStreamDelta(
  state: RenderState,
  stream: 'assistant' | 'thinking',
  label: string,
  tone: Tone,
  delta: string,
): void {
  if (state.activeStream !== stream) {
    finishStream(state)
    state.output.write(`\n${color(label, tone, state.output)}`)
    state.activeStream = stream
    state.streamAtLineStart = false
  }
  state.output.write(stream === 'thinking' ? color(delta, 'dim', state.output) : delta)
  state.streamAtLineStart = delta.endsWith('\n')
}

export function finishStream(state: RenderState): void {
  if (state.activeStream && !state.streamAtLineStart) state.output.write('\n')
  state.activeStream = null
  state.streamAtLineStart = true
}

function formatUsage(usage: TokenUsage): string {
  return [
    `in=${usage.inputTokens}`,
    `out=${usage.outputTokens}`,
    `cache_read=${usage.cacheReadTokens}`,
    `cache_write=${usage.cacheWriteTokens}`,
  ].join(' ')
}

function formatToolInput(block: Extract<Block, { type: 'tool_call' }>): string {
  try {
    const input = JSON.parse(block.input) as Record<string, unknown>
    if (typeof input.description === 'string' && input.description.trim()) {
      return trimOneLine(input.description)
    }
    if (block.toolName === 'shell_exec' && typeof input.script === 'string') {
      return trimOneLine(input.script)
    }
    if (block.toolName === 'shell_status' && typeof input.commandId === 'string') return `Check ${input.commandId}`
    if (block.toolName === 'shell_write' && typeof input.commandId === 'string') return `Send input to ${input.commandId}`
    if (block.toolName === 'shell_abort' && typeof input.commandId === 'string') return `Stop ${input.commandId}`
    if (block.toolName === 'yield' && typeof input.durationMs === 'number') return `Wait ${input.durationMs}ms`
  } catch {
    // Fall through to raw input.
  }
  return trimOneLine(block.input)
}

function trimOneLine(text: string): string {
  const compact = text.replace(/\s+/g, ' ').trim()
  return compact.length > 100 ? `${compact.slice(0, 97)}...` : compact
}

function formatUserContent(content: UserContentBlock[]): string {
  return trimOneLine(content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join(' '))
}
