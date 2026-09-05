import type { ProviderErrorDiagnostics } from '@demicodes/core'
import { t } from '@demicodes/web-ui/infra/i18n'

/** One plain sentence for the chrome, from the normalized code. The upstream text goes in the body. */
export function errorSummary(code: string | null | undefined): string {
  switch (code) {
    case 'rate_limit': return t('agent.error.rateLimit')
    case 'overloaded': return t('agent.error.overloaded')
    case 'auth_expired': return t('agent.error.authExpired')
    case 'auth_missing':
    case 'credential_not_found': return t('agent.error.authMissing')
    case 'context_length_exceeded': return t('agent.error.contextLength')
    default: return t('agent.error.failed')
  }
}

/** The short facts a support thread asks for first, in one line under the upstream message. */
export function errorFacts(code: string | null | undefined, diagnostics: ProviderErrorDiagnostics | undefined): string[] {
  const facts: string[] = []
  if (diagnostics?.httpStatus !== undefined) facts.push(`HTTP ${diagnostics.httpStatus}`)
  if (code) facts.push(code)
  if (diagnostics?.clientRequestId) facts.push(diagnostics.clientRequestId)
  return facts
}

/** What the copy button puts on the clipboard: the upstream message and every diagnostic. */
export function errorReportText(message: string, code: string | null | undefined, diagnostics: ProviderErrorDiagnostics | undefined): string {
  const lines = [message]
  if (code) lines.push(`code: ${code}`)
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
      if (value !== undefined && value !== '') lines.push(`${key}: ${value}`)
    }
  }
  return lines.join('\n')
}
