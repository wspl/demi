import { z } from 'zod'

/**
 * What of a received value did not read with its schema, in words that
 * carry no value of it, as the page reports its own failures
 * (`crates/backend-http/src/request_id.rs`): each issue's path with what
 * was expected, and for a wrong type the type that came. zod's own messages
 * name types, and a schema's limits and options; a refinement says its
 * schema's own fixed words, which never quote the value. The names of keys
 * the schema does not know come from the value, so only their count is
 * said.
 */
export function schemaRefusal(error: z.ZodError): string {
  const issues = error.issues.map((issue) => {
    switch (issue.code) {
      case 'unrecognized_keys':
        return { ...issue, keys: [], message: `${issue.keys.length} unrecognized ${issue.keys.length === 1 ? 'key' : 'keys'}` }
      default:
        return issue
    }
  })
  return z.prettifyError(new z.ZodError(issues))
}
