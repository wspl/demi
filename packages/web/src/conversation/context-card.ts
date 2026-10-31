import type { InstructionEntry } from '@demicodes/protocol'
import type { UsageLimits } from '@demicodes/web-ui/agent/context-usage'
import type { MessageFileTarget } from '@demicodes/web-ui/markdown/types'
import { useResources } from '../state/resources'
import { useSettingsAddress } from '../settings/address'
import { useProviderSettings } from '../settings/providers'

/**
 * What a context card needs of the product beside its usage, the composer's
 * for the root and the Agents window's for a child alike: the usage limits
 * of the account a provider entry runs on, their refresh, and where an
 * instruction opens. `openFile` shows a Host file in the work panel; absent
 * while no plugin shows files.
 */
export function useContextCard(openFile: () => ((target: MessageFileTarget) => void) | undefined) {
  const resources = useResources()
  const settings = useSettingsAddress()
  const providerSettings = useProviderSettings()

  /**
   * The subscription account the next request of an agent of the entry
   * `providerId` runs on: under account rotation, `account`, the one its
   * context usage names, which the agent takes ahead
   * (`crates/provider-registry/src/rotation.rs`), and none until it took one;
   * under Manual, the entry's active account. None on an API key.
   */
  function accountOf(providerId: string, account: string | null) {
    const provider = resources.providers.find((entry) => entry.id === providerId)
    if (provider?.kind !== 'subscription') {
      return null
    }
    const used = provider.rotation === 'manual'
      ? provider.accounts.find((entry) => entry.active)
      : provider.accounts.find((entry) => entry.id === account)
    return used ? { provider, account: used } : null
  }

  /** What the card shows of that account's usage limits (`crates/backend-providers/src/usage/mod.rs`). */
  function limitsOf(providerId: string, account: string | null): UsageLimits | null {
    const current = accountOf(providerId, account)
    return current
      ? {
          accountLabel: current.account.label,
          windows: current.account.quota,
          observedAt: current.account.quotaAt ?? null,
          refreshing: providerSettings.refreshingUsage[current.provider.id]?.[current.account.id] === true,
          error: providerSettings.usageFailures[current.provider.id]?.[current.account.id] ?? null,
        }
      : null
  }

  /** The card's Refresh usage button: the store's request, as Settings asks. */
  function refreshLimits(providerId: string, account: string | null): void {
    const current = accountOf(providerId, account)
    if (current) {
      void providerSettings.refreshUsage(current.provider, current.account.id)
    }
  }

  /** The personal instructions open their settings; a file shows in the work panel. */
  function openInstruction(entry: InstructionEntry): void {
    if (entry.kind === 'personal') {
      void settings.open('instructions')
    } else {
      openFile()?.({ path: entry.path, line: null })
    }
  }

  return { limitsOf, refreshLimits, openInstruction }
}
