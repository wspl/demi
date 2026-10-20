import type { InstructionEntry } from '@demicodes/protocol'
import type { UsageLimits } from '@demicodes/web-ui/agent/context-usage'
import type { MessageFileTarget } from '@demicodes/web-ui/markdown/types'
import { useResources } from '../state/resources'
import { useSettingsAddress } from '../settings/address'
import { useProviderSettings } from '../settings/providers'

/**
 * What a context card needs of the product beside its usage, the composer's
 * for the root and the Agents window's for a child alike
 * (`compaction.md` § What fills the context): the usage limits of the
 * account a provider entry runs on, their refresh, and where an instruction
 * opens. `openFile` shows a Host file in the work panel; absent while no
 * plugin shows files.
 */
export function useContextCard(openFile: () => ((target: MessageFileTarget) => void) | undefined) {
  const resources = useResources()
  const settings = useSettingsAddress()
  const providerSettings = useProviderSettings()

  /**
   * The subscription account an agent of the entry `providerId` runs on:
   * `account`, the one its context usage names under account rotation
   * (`crates/backend-providers/src/llm/rotation.rs`), else the entry's active account;
   * none on an API key.
   */
  function accountOf(providerId: string, account: string | null) {
    const provider = resources.providers.find((entry) => entry.id === providerId)
    const accounts = provider?.kind === 'subscription' ? provider.accounts : []
    const used = accounts.find((entry) => entry.id === account) ?? accounts.find((entry) => entry.active)
    return provider && used ? { provider, account: used } : null
  }

  /** What the card shows of that account's usage limits (`crates/backend-providers/src/usage/mod.rs`). */
  function limitsOf(providerId: string, account: string | null): UsageLimits | null {
    const current = accountOf(providerId, account)
    return current
      ? {
          accountLabel: current.account.label,
          windows: current.account.quota,
          autoRefresh: current.provider.autoRefreshUsage ?? false,
          refreshing: providerSettings.refreshingUsage[current.provider.id]?.[current.account.id] === 'manual',
          error: providerSettings.usageFailures[current.provider.id]?.[current.account.id] ?? null,
        }
      : null
  }

  /** The card showed the limits (automatic, at most once a minute per account) or its Refresh usage button was pressed: the store's request, as Settings asks. */
  function refreshLimits(providerId: string, account: string | null, automatic: boolean): void {
    const current = accountOf(providerId, account)
    if (current) {
      void providerSettings.refreshUsage(current.provider, current.account.id, automatic)
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
