import type { AgentProvider, InferenceRequest, ProviderEvent } from '@demicodes/provider'

/**
 * The scripted model behind the world's `stub` provider type. Scripts are
 * queued per agent session (the conversation id) one turn at a time, so a
 * scenario reads linearly: script the turn, send, observe. A request from a
 * session nothing was scripted for is a subagent child and consumes the
 * shared child queue.
 *
 * A turn's script is a list of provider events or a function of the
 * inference request, which is how a scenario asserts what the model was
 * shown. Every request the model answered is kept in `requests`, in order.
 */
export type TurnScript = ProviderEvent[] | ((request: InferenceRequest) => ProviderEvent[] | AsyncIterable<ProviderEvent>)

export class ScriptedModel {
  private readonly queues = new Map<string, TurnScript[]>()
  private readonly children: TurnScript[] = []
  readonly requests: InferenceRequest[] = []
  /** Requests whose script ran to its `response`; an abort cuts a script short and leaves no usage. */
  answered = 0

  script(sessionId: string, ...turns: TurnScript[]): void {
    const queue = this.queues.get(sessionId) ?? []
    queue.push(...turns)
    this.queues.set(sessionId, queue)
  }

  scriptChild(...turns: TurnScript[]): void {
    this.children.push(...turns)
  }

  /** Drops what a session had left to say: its turn was cut short and the script with it. */
  clear(sessionId: string): void {
    this.queues.delete(sessionId)
  }

  /** Scripts left unconsumed, for the teardown check. */
  pending(): string[] {
    const left = [...this.queues].filter(([, queue]) => queue.length > 0).map(([id, queue]) => `${id}: ${queue.length}`)
    if (this.children.length > 0) left.push(`children: ${this.children.length}`)
    return left
  }

  /** One runtime serves every session: the queue is chosen per request. */
  runtime(): AgentProvider {
    const model = this
    const runtime: AgentProvider = {
      async *run(request) {
        model.requests.push(request)
        const queue = model.queues.get(request.sessionId)
        const turn = queue?.length ? queue.shift() : model.children.shift()
        if (turn === undefined) throw new Error(`ScriptedModel: nothing scripted for session ${request.sessionId} (request #${model.requests.length})`)
        const events = typeof turn === 'function' ? turn(request) : turn
        for await (const event of events) yield event
        model.answered += 1
      },
      clone: () => runtime,
    }
    return runtime
  }
}

/** The text content of every block of the given items, joined. */
export function itemsText(items: InferenceRequest['items']): string {
  const parts: string[] = []
  for (const item of items) {
    if (item.type === 'user_message' || item.type === 'user_steer') {
      for (const block of item.content) if (block.type === 'text') parts.push(block.text)
    } else if (item.type === 'assistant_text') parts.push(item.text)
    else if (item.type === 'tool_result') for (const block of item.output) if (block.type === 'text') parts.push(block.text)
  }
  return parts.join('\n')
}
