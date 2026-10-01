import { z } from 'zod'

/**
 * What a scripted reply holds, block by block: text the model writes, or a
 * tool it calls with this input.
 */
export type ScriptedBlock =
  | { type: 'text'; text: string }
  | { type: 'tool_use'; name: string; input: Record<string, unknown> }

/**
 * A system prompt as the Messages API takes it, a string or text blocks
 * (which the backend sends, so that a block can carry a cache mark), read as
 * its text.
 */
const systemSchema = z
  .union([z.string(), z.array(z.object({ type: z.literal('text'), text: z.string() }))])
  .transform((system) => (typeof system === 'string' ? system : system.map((block) => block.text).join('')))

/**
 * The parts of a Messages API request the suite reads (the backend's
 * Anthropic provider writes the rest).
 */
const messagesRequestSchema = z.object({
  model: z.string(),
  system: systemSchema.optional(),
  messages: z.array(z.object({ role: z.enum(['user', 'assistant']), content: z.unknown() })),
  stream: z.literal(true),
})
export type MessagesRequest = z.infer<typeof messagesRequestSchema>

/**
 * A reply the endpoint holds: it answers the request only on `release`, so
 * the turn stays in its provider stream while a test acts on it.
 */
export interface HeldReply {
  /** Settles once the request the reply answers has arrived. */
  requested: Promise<void>
  /** Settles if the backend cancels that request before the release. */
  cancelled: Promise<void>
  release(): void
}

/** A scripted reply, with its hold when it has one. */
interface Reply {
  blocks: ScriptedBlock[]
  hold?: {
    requested: PromiseWithResolvers<void>
    cancelled: PromiseWithResolvers<void>
    released: PromiseWithResolvers<void>
  }
}

/** The system prompt of a title request (`agent::title::TITLE_INSTRUCTION` begins so). */
const TITLE_REQUEST = 'You are a title generator.'

/**
 * An Anthropic-compatible Messages endpoint the suite scripts
 * (`scenarios.md` § Web app contract suite). The backend's Anthropic API
 * provider calls it as it calls the vendor: each turn request is answered
 * with the next scripted reply as the vendor's event stream, a title request
 * with the next scripted title, and every request is recorded, so a test
 * reads what the model received. No request reaches a real model.
 */
export function startScriptedAnthropic() {
  const replies: Reply[] = []
  /** The holds not yet released, which `clear` releases so that no answer waits forever. */
  const holding = new Set<() => void>()
  const titles: string[] = []
  const turns: MessagesRequest[] = []
  let toolUses = 0

  function stream(blocks: readonly ScriptedBlock[]): Response {
    const events: object[] = [{
      type: 'message_start',
      message: {
        id: `msg_${turns.length}`, type: 'message', role: 'assistant', model: 'scripted', content: [],
        usage: { input_tokens: 10, output_tokens: 1 },
      },
    }]
    blocks.forEach((block, index) => {
      if (block.type === 'text') {
        // Two deltas, so the page applies an appended text as well as a new block.
        const half = Math.ceil(block.text.length / 2)
        events.push(
          { type: 'content_block_start', index, content_block: { type: 'text', text: '' } },
          { type: 'content_block_delta', index, delta: { type: 'text_delta', text: block.text.slice(0, half) } },
          { type: 'content_block_delta', index, delta: { type: 'text_delta', text: block.text.slice(half) } },
        )
      } else {
        toolUses += 1
        events.push(
          { type: 'content_block_start', index, content_block: { type: 'tool_use', id: `toolu_${toolUses}`, name: block.name, input: {} } },
          { type: 'content_block_delta', index, delta: { type: 'input_json_delta', partial_json: JSON.stringify(block.input) } },
        )
      }
      events.push({ type: 'content_block_stop', index })
    })
    const calls = blocks.some((block) => block.type === 'tool_use')
    events.push(
      { type: 'message_delta', delta: { stop_reason: calls ? 'tool_use' : 'end_turn' }, usage: { output_tokens: 5 } },
      { type: 'message_stop' },
    )
    const body = events.map((event) => `event: ${'type' in event ? String(event.type) : 'message'}\ndata: ${JSON.stringify(event)}\n\n`).join('')
    return new Response(body, { headers: { 'Content-Type': 'text/event-stream' } })
  }

  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      if (new URL(request.url).pathname !== '/v1/messages' || request.method !== 'POST') {
        return new Response('Not found', { status: 404 })
      }
      const parsed = messagesRequestSchema.safeParse(await request.json())
      if (!parsed.success) {
        return Response.json({ type: 'error', error: { type: 'invalid_request_error', message: z.prettifyError(parsed.error) } }, { status: 400 })
      }
      if (parsed.data.system?.startsWith(TITLE_REQUEST)) {
        return stream([{ type: 'text', text: titles.shift() ?? 'Scripted conversation' }])
      }
      turns.push(parsed.data)
      const reply = replies.shift()
      if (!reply) {
        return Response.json({ type: 'error', error: { type: 'api_error', message: 'The script has no reply for this request' } }, { status: 500 })
      }
      const hold = reply.hold
      if (hold) {
        // A cancelled request is answered as well: the answer goes nowhere,
        // and the handler ends.
        request.signal.addEventListener('abort', () => hold.cancelled.resolve(), { once: true })
        hold.requested.resolve()
        await Promise.race([hold.released.promise, hold.cancelled.promise])
      }
      return stream(reply.blocks)
    },
  })

  /** Drops the replies not yet asked for, and releases every hold. */
  function clear(): void {
    replies.length = 0
    titles.length = 0
    for (const release of [...holding]) {
      release()
    }
  }

  return {
    /** The base URL an entry names: the provider appends `/messages`. */
    url: `http://127.0.0.1:${server.port}/v1`,
    /** Queues the reply of the next turn request. */
    reply(...blocks: ScriptedBlock[]): void {
      replies.push({ blocks })
    },
    /** Queues the reply of the next turn request, held until its release. */
    hold(...blocks: ScriptedBlock[]): HeldReply {
      const hold = {
        requested: Promise.withResolvers<void>(),
        cancelled: Promise.withResolvers<void>(),
        released: Promise.withResolvers<void>(),
      }
      const release = () => {
        holding.delete(release)
        hold.released.resolve()
      }
      holding.add(release)
      replies.push({ blocks, hold })
      return { requested: hold.requested.promise, cancelled: hold.cancelled.promise, release }
    },
    /** Queues the answer of the next title request. */
    title(text: string): void {
      titles.push(text)
    },
    /** The turn requests the endpoint received, oldest first. */
    turns: (): readonly MessagesRequest[] => turns,
    /** The replies not yet asked for. */
    unused: (): number => replies.length,
    clear,
    async stop(): Promise<void> {
      clear()
      await server.stop(true)
    },
  }
}

export type ScriptedAnthropic = ReturnType<typeof startScriptedAnthropic>
