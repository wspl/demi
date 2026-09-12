import { z } from 'zod'

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
