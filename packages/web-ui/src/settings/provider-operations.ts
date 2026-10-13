import type { Operation } from '../ui/operations'
import type { SettingsProviderOperation } from './types'

/** A provider entry as the target of operations (`operations.ts`). */
export function providerTarget(id: string): string {
  return `provider:${id}`
}

/**
 * A provider's operation (`SettingsProviderOperation`) as the page keeps it:
 * every one is of one group, so a provider does one thing at a time, as its
 * writes go one at a time.
 */
export function providerOperation(id: string, operation: SettingsProviderOperation): Operation {
  return {
    action: operation.kind === 'account' ? `account-${operation.action}:${operation.accountId}` : operation.kind,
    target: providerTarget(id),
    group: 'provider',
  }
}
