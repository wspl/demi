import type { ProviderEntry } from './providers'
import type { PublicProvider } from '@demicodes/product-contracts'

export function publicProvider(provider: ProviderEntry): PublicProvider {
  const { config } = provider
  const keyed = config.kind === 'api_key' ? config : null
  return {
    id: provider.id,
    kind: config.kind,
    providerType: config.providerType,
    label: provider.label,
    wireApi: keyed?.wireApi ?? null,
    vendorId: keyed?.vendorId ?? null,
    baseUrl: keyed?.baseUrl ?? null,
    models: keyed?.models ?? null,
    keyConfigured: keyed !== null,
    createdAt: provider.createdAt,
  }
}
