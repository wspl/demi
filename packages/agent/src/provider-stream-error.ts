/** Error carrying a provider stream's error code (e.g. `context_length_exceeded`). */
export class ProviderStreamError extends Error {
  readonly code: string | null

  constructor(message: string, code: string | null) {
    super(message)
    this.name = 'ProviderStreamError'
    this.code = code
  }
}

/** Whether an error is a provider stream error reporting the context window was exceeded. */
export function isContextLengthExceeded(error: unknown): boolean {
  return error instanceof ProviderStreamError && error.code === 'context_length_exceeded'
}
