import type { ProviderErrorDiagnostics } from '@demicodes/core'
import { z } from 'zod'

/** A first line stays one line: past this the source's text goes in the body. */
const HEADLINE_MAX_LENGTH = 160

/** What a failure record shows: the first line, and the body when the first line is not the whole story. */
export interface ErrorPresentation {
  label: string
  detail: string | null
}

const vendorErrorSchema = z.object({
  error: z.union([
    z.string(),
    z.object({ message: z.string() }),
  ]).optional(),
  message: z.string().optional(),
})

/** The sentence a vendor put in its JSON error body, from the shapes vendors share; null when there is none. */
function vendorSentence(message: string): string | null {
  const start = message.indexOf('{')
  if (start === -1)
    return null
  let body: unknown
  try {
    body = JSON.parse(message.slice(start))
  } catch {
    return null
  }
  const parsed = vendorErrorSchema.safeParse(body)
  if (!parsed.success)
    return null
  const error = parsed.data.error
  const sentence = typeof error === 'string' ? error : error?.message ?? parsed.data.message
  return sentence?.trim() || null
}

/**
 * The first line of a failure is what its source said, never Demi's reading of
 * it. A plain sentence is the whole record. A message that wraps a vendor's
 * JSON body leads with the sentence inside it and keeps the full text below.
 * Only a message with no sentence to lead with falls back to a neutral line.
 */
export function errorPresentation(message: string): ErrorPresentation {
  const text = message.trim()
  const sentence = vendorSentence(text)
  if (sentence !== null && sentence.length <= HEADLINE_MAX_LENGTH && !sentence.includes('\n'))
    return { label: sentence, detail: text }
  if (text.length > 0 && text.length <= HEADLINE_MAX_LENGTH && !text.includes('\n'))
    return { label: text, detail: null }
  return { label: 'The turn failed', detail: text || null }
}

/**
 * The line under a failure: when the provider says it works again, which is
 * what the reader does next. The status, the normalized code and the request
 * ids are bookkeeping for a support thread; they go in the copied report.
 */
export function errorFacts(retryAt: string | null): string[] {
  return retryAt === null ? [] : [`resets ${new Date(retryAt).toLocaleString()}`]
}

/** What the copy button puts on the clipboard: the upstream message and every diagnostic. */
export function errorReportText(
  message: string,
  code: string | null | undefined,
  diagnostics: ProviderErrorDiagnostics | undefined
): string {
  const lines = [message]
  if (code)
    lines.push(`code: ${code}`)
  if (diagnostics) {
    const fields: Array<[string, string | number | undefined]> = [
      ['source', diagnostics.source],
      ['http', diagnostics.httpStatus],
      ['provider code', diagnostics.providerCode],
      ['request', diagnostics.clientRequestId],
      ['provider request', diagnostics.providerRequestId],
      ['response', diagnostics.providerResponseId],
    ]
    for (const [key, value] of fields) {
      if (value !== undefined && value !== '')
        lines.push(`${key}: ${value}`)
    }
    if (diagnostics.upstream)
      lines.push('', 'upstream:', prettyUpstream(diagnostics.upstream))
  }
  return lines.join('\n')
}

/** The provider's response as it arrived, indented when it is JSON, for a reader's eyes. */
export function prettyUpstream(upstream: string): string {
  try {
    return JSON.stringify(JSON.parse(upstream), null, 2)
  } catch {
    return upstream
  }
}
