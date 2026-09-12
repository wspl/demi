import { z } from 'zod'
import { base64ToBytes, decodeUtf8Strict } from '@demicodes/utils'

/** A malformed provider payload, with diagnostics that exclude its values. */
export class ProviderDataError extends Error {
  readonly code = 'invalid_provider_response'

  constructor(source: string, detail: string) {
    super(`${source}: ${detail}`)
    this.name = 'ProviderDataError'
  }
}

export function parseProviderData<T extends z.core.$ZodType>(
  schema: T,
  value: unknown,
  source: string
): z.core.output<T> {
  const result = z.safeParse(schema, value)
  if (!result.success) {
    const fields = [...new Set(issueFields(result.error.issues))]
    throw new ProviderDataError(source, `invalid fields: ${fields.join(', ')}`)
  }
  return result.data
}

function issueFields(issues: readonly z.core.$ZodIssue[]): string[] {
  return issues.flatMap((issue) => {
    if (issue.code === 'invalid_union') {
      return issue.errors.flatMap(issueFields)
    }
    return [issue.path.join('.') || 'payload']
  })
}

/** JSON syntax is decoded before the receiving schema validates structure. */
export function parseProviderJson<T extends z.core.$ZodType>(
  schema: T,
  text: string,
  source: string
): z.core.output<T> {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    throw new ProviderDataError(source, 'invalid JSON')
  }
  return parseProviderData(schema, value, source)
}

/** Decodes metadata claims only; it does not verify a JWT signature. */
export function parseProviderJwt<T extends z.core.$ZodType>(
  schema: T,
  jwt: string,
  source: string,
): z.core.output<T> | null {
  const parts = jwt.split('.')
  if (parts.length !== 3) {
    // Opaque access tokens do not expose claims.
    return null
  }
  const payload = parts[1]
  if (!payload || !/^[A-Za-z0-9_-]+$/.test(payload) || payload.length % 4 === 1) {
    throw new ProviderDataError(source, 'invalid payload encoding')
  }
  let json: string | null
  try {
    const base64 = payload.replace(/-/g, '+').replace(/_/g, '/')
    const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, '=')
    json = decodeUtf8Strict(base64ToBytes(padded))
  } catch {
    throw new ProviderDataError(source, 'invalid payload encoding')
  }
  if (json === null) {
    throw new ProviderDataError(source, 'invalid UTF-8 payload')
  }
  return parseProviderJson(schema, json, source)
}
