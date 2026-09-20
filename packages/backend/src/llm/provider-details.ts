import type { Provider, ProviderQuotaSnapshot } from '@demicodes/provider'

/**
 * Public account/runtime facts, never raw vendor envelopes or stored credential
 * material. `usage` is each account's kept snapshot; without it only the
 * provider's own account reports usage.
 */
export async function providerDetails(
  provider: Provider,
  options: {
    requireAccount?: boolean
    usage?: ReadonlyMap<string, ProviderQuotaSnapshot | null>
  } = {}
) {
  const listed = await provider.credentials?.list() ?? []
  const missingAccount = options.requireAccount === true && listed.length === 0
  const [auth, runtime, active] = await Promise.all([
    missingAccount ? {
      status: 'unauthenticated' as const,
      message: 'No subscription account configured'
    } : provider.auth?.status() ?? { status: 'unknown' as const },
    provider.state?.() ?? { status: 'unknown' as const },
    missingAccount ? null : provider.credentials?.getActive() ?? null,
  ])
  const own = provider.quota?.latest() ?? null
  const usageOf = (credentialId: string) => options.usage
    ? options.usage.get(credentialId) ?? null
    : credentialId === active?.credentialId ? own : null
  return {
    auth,
    runtime,
    accounts: listed.map(account => ({
      ...account,
      quota: publicQuota(usageOf(account.id))
    })),
    active,
    credentials: provider.credentials?.capability() ??
      { mode: 'none' as const },
    quota: publicQuota(
      active?.credentialId ? usageOf(active.credentialId) : own
    ),
    quotaCapability: provider.quota?.capability() ?? { mode: 'none' as const },
    requiresProcessCapableHost: provider.requiresProcessCapableHost ?? false,
  }
}

/**
 * What a user who only infers with an entry may know of it
 * (providers-and-vault.md § Scope): whether it works, not whose account it is
 * or how much of it is used.
 */
export function inferenceOnlyDetails(
  details: Awaited<ReturnType<typeof providerDetails>>
): Awaited<ReturnType<typeof providerDetails>> {
  const auth = details.auth.status === 'authenticated'
    ? { status: 'authenticated' as const }
    : details.auth
  return {
    ...details,
    auth,
    accounts: [],
    active: details.active ? { credentialId: null, status: auth } : null,
    quota: null,
  }
}

export function publicQuota(snapshot: ProviderQuotaSnapshot | null) {
  if (!snapshot)
    return null
  return {
    providerId: snapshot.providerId,
    observedAt: snapshot.observedAt,
    source: snapshot.source,
    accountLabel: snapshot.accountLabel ?? null,
    plan: snapshot.plan ? {
      id: snapshot.plan.id,
      label: snapshot.plan.label ?? null
    } : null,
    windows: snapshot.windows,
  }
}
