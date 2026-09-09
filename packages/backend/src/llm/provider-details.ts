import type { Provider, ProviderQuotaSnapshot } from '@demicodes/provider'

/**
 * Public account/runtime facts, never raw vendor envelopes or stored credential
 * material.
 */
export async function providerDetails(
  provider: Provider,
  requireAccount = false
) {
  const accounts = await provider.credentials?.list() ?? []
  const missingAccount = requireAccount && accounts.length === 0
  const [auth, runtime, active] = await Promise.all([
    missingAccount ? {
      status: 'unauthenticated' as const,
      message: 'No subscription account configured'
    } : provider.auth?.status() ?? { status: 'unknown' as const },
    provider.state?.() ?? { status: 'unknown' as const },
    missingAccount ? null : provider.credentials?.getActive() ?? null,
  ])
  return {
    auth,
    runtime,
    accounts,
    active,
    credentials: provider.credentials?.capability() ??
      { mode: 'none' as const },
    quota: publicQuota(provider.quota?.latest() ?? null),
    quotaCapability: provider.quota?.capability() ?? { mode: 'none' as const },
    requiresProcessCapableHost: provider.requiresProcessCapableHost ?? false,
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
