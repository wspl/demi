import type { CatalogProvider } from './assembly'

export function modelAvailability(
  provider: CatalogProvider
) {
  if (provider.auth.status === 'unauthenticated' ||
    provider.auth.status === 'error')
    return {
      available: false,
      reason: 'authentication',
      message: 'Provider login is unavailable'
    }
  if (provider.runtime.status === 'unavailable' ||
    provider.runtime.status === 'error')
    return {
      available: false,
      reason: 'runtime',
      message: provider.runtime.message
    }
  return { available: true, reason: null, message: null }
}
