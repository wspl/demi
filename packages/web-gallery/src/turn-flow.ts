import { nextTick, onBeforeUnmount, ref } from 'vue'
import { t } from '@demicodes/web-ui/infra/i18n'
import type { ActivityKind } from './components/ActivitySlot.vue'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { CHROME_ROLL_MS } from '@demicodes/web-ui/ui/chrome-roll'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'
import { demoModel } from './fixtures/blocks'

export type { ActivityKind }

export interface ActivitySlotState {
  kind: ActivityKind
  label: string
  incoming?: MessageListBlock | null
}

/** `stream` is thinking then reply with no activity slot: the reveal alone. */
export type TurnFlowKind = 'turn' | 'resume' | 'retry' | 'connect' | 'stream'

const THINK_1 = 'The cookie name changed from sid to session. The helper already writes the new header. The test is the one still looking for sid.'
const THINK_2 = 'The helper is fine. Update the assertion in auth.test.ts and leave cookie.ts alone.'
const REPLY = 'The cookie helper is fine. The test still expects `sid`.\n\nI updated the assertion in `auth.test.ts` and left `cookie.ts` alone.'
const HANDOFF_MS = CHROME_ROLL_MS + 80
const WAIT_MS = 80
const FEED_CHARS = 4
const FEED_MS = 90

export function useTurnFlow() {
  const blocks = ref<MessageListBlock[]>([])
  const slot = ref<ActivitySlotState | null>(null)
  const endedAtById = ref<Record<string, string>>({})
  const streamingThinkingId = ref<string | null>(null)
  const streamingTextId = ref<string | null>(null)
  const running = ref(false)
  const timers: number[] = []
  let token = 0
  let thinkStartedAt = ''

  function labelFor(kind: ActivityKind): string {
    switch (kind) {
      case 'connecting': return t('agent.block.connecting')
      case 'resuming': return t('agent.block.resuming')
      case 'retrying': return t('agent.block.retrying')
      default: return t('agent.block.requesting')
    }
  }

  function setSlot(kind: ActivityKind | null): void {
    slot.value = kind ? { kind, label: labelFor(kind) } : null
  }

  function clearTimers(): void {
    for (const id of timers) window.clearTimeout(id)
    timers.length = 0
  }

  function cancel(): void {
    token += 1
    clearTimers()
  }

  function stop(): void {
    cancel()
    running.value = false
    slot.value = null
    streamingThinkingId.value = null
    streamingTextId.value = null
  }

  function at(run: number, ms: number, fn: () => void): void {
    timers.push(window.setTimeout(() => {
      if (run !== token) return
      fn()
    }, ms))
  }

  function now(): string {
    return new Date().toISOString()
  }

  function replace(id: string, next: MessageListBlock): void {
    blocks.value = blocks.value.map((block) => (block.id === id ? next : block))
  }

  function append(block: MessageListBlock): void {
    if (blocks.value.some((existing) => existing.id === block.id)) return
    blocks.value = [...blocks.value, block]
  }

  function thinkingBlock(id: string, text: string): MessageListBlock {
    return {
      type: 'thinking',
      id,
      createdAt: thinkStartedAt || now(),
      model: demoModel,
      text,
      signature: null,
    }
  }

  function textBlock(id: string, createdAt: string, text: string): MessageListBlock {
    return {
      type: 'text',
      id,
      createdAt,
      model: demoModel,
      text,
    }
  }

  function tool(id: string, status: ToolCallBlock['status']): ToolCallBlock {
    const output = status === 'completed'
      ? [{ type: 'text' as const, text: 'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")\n' }]
      : []
    return {
      type: 'tool_call',
      id,
      createdAt: now(),
      model: demoModel,
      toolUseId: `${id}-use`,
      toolName: 'shell_exec',
      status,
      input: JSON.stringify({
        script: 'rg -n "sid" packages/web/src/auth.test.ts',
        description: 'Find the old cookie name in the login test',
      }),
      streamingOutput: [],
      output,
      view: {
        chunks: [{ stream: 'stdout', text: output[0]?.text ?? '' }],
      },
    }
  }

  function feedPrefixes(full: string): string[] {
    const units = segmentStreamUnits(full)
    const prefixes: string[] = []
    let acc = ''
    let chunk = ''
    for (const unit of units) {
      chunk += unit
      acc += unit
      if (chunk.length >= FEED_CHARS) {
        prefixes.push(acc)
        chunk = ''
      }
    }
    if (chunk || prefixes.length === 0) prefixes.push(acc)
    return prefixes
  }

  function streamTextInto(run: number, startMs: number, full: string, apply: (text: string) => void): number {
    const prefixes = feedPrefixes(full)
    prefixes.forEach((text, index) => {
      at(run, startMs + index * FEED_MS, () => apply(text))
    })
    return startMs + Math.max(0, prefixes.length - 1) * FEED_MS
  }

  function patchThinking(id: string, text: string): void {
    const next = thinkingBlock(id, text)
    replace(id, next)
    if (slot.value?.incoming?.id === id) slot.value = { ...slot.value, incoming: next }
  }

  function reveal(run: number, ms: number, build: () => MessageListBlock): void {
    at(run, ms, () => {
      const incoming = build()
      if (!slot.value) {
        append(incoming)
        if (incoming.type === 'thinking') streamingThinkingId.value = incoming.id
        return
      }
      slot.value = { ...slot.value, incoming }
      if (incoming.type === 'thinking') streamingThinkingId.value = incoming.id
    })
    at(run, ms + HANDOFF_MS, () => {
      const incoming = slot.value?.incoming
      if (incoming) append(incoming)
      slot.value = null
    })
  }

  function endThinking(id: string): void {
    endedAtById.value = { ...endedAtById.value, [id]: now() }
    if (streamingThinkingId.value === id) streamingThinkingId.value = null
  }

  function streamThinking(run: number, startMs: number, id: string, text: string): number {
    return streamTextInto(run, startMs, text, (partial) => patchThinking(id, partial))
  }

  function streamReply(run: number, startMs: number, id: string, text: string, onDone: () => void): void {
    let createdAt = ''
    at(run, startMs, () => {
      createdAt = now()
      streamingTextId.value = id
      append(textBlock(id, createdAt, ''))
    })
    const end = streamTextInto(run, startMs + FEED_MS, text, (partial) => {
      replace(id, textBlock(id, createdAt || now(), partial))
    })
    at(run, end + 160, () => {
      streamingTextId.value = null
      onDone()
    })
  }

  function playThinkingThenReply(
    run: number,
    startMs: number,
    thinkId: string,
    text: string,
    replyText = REPLY,
  ): void {
    at(run, startMs, () => {
      thinkStartedAt = now()
    })
    reveal(run, startMs, () => thinkingBlock(thinkId, ''))
    const thinkEnd = streamThinking(run, startMs + HANDOFF_MS, thinkId, text)
    at(run, thinkEnd + 200, () => {
      endThinking(thinkId)
    })
    streamReply(run, thinkEnd + 240, `${run}-text`, replyText, () => {
      running.value = false
    })
  }

  function play(kind: TurnFlowKind = 'turn', userText?: string): void {
    cancel()
    const run = token
    running.value = true
    endedAtById.value = {}
    streamingThinkingId.value = null
    streamingTextId.value = null
    thinkStartedAt = ''
    slot.value = null

    if (kind === 'stream') {
      blocks.value = []
      playThinkingThenReply(run, 0, `${run}-think`, THINK_1)
      return
    }

    if (kind === 'connect') {
      blocks.value = []
      void nextTick(() => {
        if (run !== token) return
        setSlot('connecting')
      })
      at(run, 800, () => setSlot('requesting'))
      playThinkingThenReply(run, 800 + WAIT_MS, `${run}-think`, THINK_2)
      return
    }

    if (kind === 'resume') {
      blocks.value = []
      void nextTick(() => {
        if (run !== token) return
        setSlot('resuming')
      })
      at(run, 800, () => setSlot('requesting'))
      playThinkingThenReply(run, 800 + WAIT_MS, `${run}-think`, THINK_2)
      return
    }

    if (kind === 'retry') {
      blocks.value = [{
        type: 'error',
        id: `${run}-error`,
        createdAt: now(),
        model: demoModel,
        message: 'Anthropic API request failed with HTTP 529: Overloaded. The upstream service is temporarily unavailable.',
        code: 'overloaded',
        diagnostics: { source: 'http', httpStatus: 529, providerCode: 'overloaded_error' },
      }]
      void nextTick(() => {
        if (run !== token) return
        setSlot('retrying')
      })
      at(run, 900, () => setSlot('requesting'))
      playThinkingThenReply(run, 900 + WAIT_MS, `${run}-think`, THINK_2)
      return
    }

    blocks.value = [{
      type: 'user',
      id: `${run}-user`,
      turnId: `turn-${run}`,
      createdAt: now(),
      model: demoModel,
      content: [{ type: 'text', text: userText?.trim() || 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.' }],
      preamble: null,
    }]
    void nextTick(() => {
      if (run !== token) return
      setSlot('requesting')
    })

    const think1 = `${run}-think-1`
    const toolId = `${run}-tool`
    const think2 = `${run}-think-2`

    at(run, 1000, () => {
      thinkStartedAt = now()
    })
    reveal(run, 1000, () => thinkingBlock(think1, ''))
    const think1End = streamThinking(run, 1000 + HANDOFF_MS, think1, THINK_1)
    at(run, think1End + 200, () => {
      endThinking(think1)
      setSlot('requesting')
    })
    reveal(run, think1End + 200 + WAIT_MS, () => tool(toolId, 'executing'))

    const afterToolArrive = think1End + 200 + WAIT_MS + HANDOFF_MS
    at(run, afterToolArrive + 1400, () => {
      replace(toolId, tool(toolId, 'completed'))
      setSlot('requesting')
    })
    at(run, afterToolArrive + 1400 + WAIT_MS, () => {
      thinkStartedAt = now()
    })
    reveal(run, afterToolArrive + 1400 + WAIT_MS, () => thinkingBlock(think2, ''))
    const think2End = streamThinking(run, afterToolArrive + 1400 + WAIT_MS + HANDOFF_MS, think2, THINK_2)
    at(run, think2End + 200, () => {
      endThinking(think2)
    })
    streamReply(run, think2End + 240, `${run}-text`, REPLY, () => {
      running.value = false
    })
  }

  onBeforeUnmount(stop)

  return { blocks, slot, endedAtById, streamingThinkingId, streamingTextId, running, play, stop }
}
