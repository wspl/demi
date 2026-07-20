import type { ProviderErrorDiagnostics } from '@demicodes/core'

/** Error carrying a provider stream's normalized code and bounded diagnostics. */
export class ProviderStreamError extends Error {
  readonly code: string | null
  readonly diagnostics: ProviderErrorDiagnostics | undefined

  constructor(message: string, code: string | null, diagnostics?: ProviderErrorDiagnostics) {
    super(message)
    this.name = 'ProviderStreamError'
    this.code = code
    this.diagnostics = diagnostics
  }
}

/** Whether an error is a provider stream error reporting the context window was exceeded. */
export function isContextLengthExceeded(error: unknown): boolean {
  return error instanceof ProviderStreamError && error.code === 'context_length_exceeded'
}
