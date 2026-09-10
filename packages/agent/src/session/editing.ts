import { stringifyPortableJson, throwIfAborted } from '@demicodes/utils'
import type { ModelSelection } from '@demicodes/core'
import type { EditRequest } from '../protocol/schemas'
import type { TranscriptLog } from '../transcript/transcript'
import type { AgentHarnessRuntime, AgentMetadata } from '../types'

/** The parsed request fixes field order and includes complete attachment bytes. */
export async function editRequestDigest(request: EditRequest): Promise<string> {
  const bytes = new TextEncoder().encode(stringifyPortableJson(request))
  const hash = await crypto.subtle.digest('SHA-256', bytes)
  return Array.from(new Uint8Array(hash), (byte) =>
    byte.toString(16).padStart(2, '0'),
  ).join('')
}

export async function prepareTranscriptEdit<State>(options: {
  runtime: AgentHarnessRuntime<State>
  transcript: TranscriptLog
  liveState: State
  request: EditRequest
  turnId: string
  model: ModelSelection
  agentSessionId: string
  cwd: string
  metadata: AgentMetadata | null
  signal: AbortSignal
}): Promise<{ state: State; applyState: () => void }> {
  const { runtime, transcript, signal } = options
  if (!runtime.restoreState) {
    throw new Error('This agent harness does not support message editing')
  }
  const prefixLength = transcript.blocks.length
  const prefix = stringifyPortableJson(transcript.blocks)
  const context = {
    agentSessionId: options.agentSessionId,
    cwd: options.cwd,
    transcript,
    metadata: options.metadata,
  }
  const state = structuredClone(await runtime.restoreState(context))
  throwIfAborted(signal)
  const content = structuredClone(options.request.content)
  await runtime.lifecycle?.({
    type: 'before_round_start',
    ...context,
    state,
    content,
  })
  throwIfAborted(signal)
  const resolved = runtime.resolveReferences
    ? await runtime.resolveReferences({ ...context, state, signal }, content)
    : content
  throwIfAborted(signal)
  const preamble = await runtime.preamble?.({ ...context, state }) ?? null
  throwIfAborted(signal)
  if (stringifyPortableJson(transcript.blocks.slice(0, prefixLength)) !== prefix) {
    throw new Error('An edit preparation hook changed the retained transcript')
  }
  transcript.pushUserTurn(
    options.turnId,
    options.model,
    structuredClone(options.request.content),
    preamble,
    false,
    structuredClone(resolved),
  )
  const acceptedState = structuredClone(state)
  return {
    state: acceptedState,
    applyState: prepareStateReplacement(options.liveState, acceptedState),
  }
}

/**
 * Harness command closures share the root state record with AgentSession.
 * Validate replacement before persistence, then keep that root identity on commit.
 * Nested state must be read through the root by consumers after each rewrite.
 */
function prepareStateReplacement(current: unknown, candidate: unknown): () => void {
  assertStateRecord(current)
  assertStateRecord(candidate)
  return () => {
    for (const key of Object.keys(current)) {
      delete current[key]
    }
    for (const key of Object.keys(candidate)) {
      Object.defineProperty(current, key, {
        value: candidate[key],
        writable: true,
        enumerable: true,
        configurable: true,
      })
    }
  }
}

function assertStateRecord(value: unknown): asserts value is Record<string, unknown> {
  if (!value || typeof value !== 'object'
    || Object.getPrototypeOf(value) !== Object.prototype
    || !Object.isExtensible(value)
    || Reflect.ownKeys(value).some((key) => {
      const descriptor = Object.getOwnPropertyDescriptor(value, key)!
      return typeof key !== 'string' || !descriptor.configurable
        || !descriptor.enumerable || !descriptor.writable || !('value' in descriptor)
    })) {
    throw new Error('Message editing requires a mutable plain record for harness state')
  }
}
