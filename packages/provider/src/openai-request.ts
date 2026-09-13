/**
 * The request side of the OpenAI wire formats, shared by every adapter that
 * builds one: Chat Completions and Responses spell these two fields the same
 * way, so they are stated once here.
 */
import type { InferenceRequest } from './types'

/**
 * The `reasoning_effort` a request asks for. These endpoints level thinking by
 * effort only: a disabled or budget-shaped thinking config names no effort, so
 * the field is left out of the body.
 */
export function thinkingToReasoningEffort(
  request: InferenceRequest
): string | undefined {
  const thinking = request.thinking
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget')
    return undefined
  return thinking.effort
}

/** Tool-call arguments as the wire spells them: a JSON string. */
export function stringifyToolArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}
