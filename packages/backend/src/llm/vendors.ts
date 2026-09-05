import {
  fetchModelsDev,
  modelListFromModelsDev,
  type ModelsDevFetch,
  type ModelsDevProvider,
  type ProviderModelList,
} from '@demicodes/provider'

/** The protocol a vendor speaks, as one of our runtime families. */
export interface VendorFamily {
  providerType: 'openai' | 'anthropic' | 'google'
  wireApi?: 'responses' | 'chat-completions'
}

/**
 * models.dev tags each vendor with the client package it is written for
 * (`npm`) — the catalog's only protocol tag. These are the packages whose
 * protocol one of our runtimes speaks; a vendor on any other package is
 * not offered.
 */
const FAMILY_BY_NPM: Record<string, VendorFamily> = {
  '@ai-sdk/openai-compatible': { providerType: 'openai', wireApi: 'chat-completions' },
  '@ai-sdk/openai': { providerType: 'openai', wireApi: 'responses' },
  '@ai-sdk/anthropic': { providerType: 'anthropic' },
  '@ai-sdk/google': { providerType: 'google' },
}

/** Vendors whose endpoint needs an auth scheme of its own, which an API key cannot satisfy. */
const NOT_OFFERED = new Set(['github-copilot'])

/** A vendor the providers page can add: its family, prefilled endpoint, and where its docs are. */
export interface Vendor extends VendorFamily {
  id: string
  name: string
  baseUrl: string | null
  doc: string | null
}

/**
 * The vendor catalog over models.dev: the vendors our runtimes can speak
 * to, and each one's live model list. Nothing here is stored — an entry
 * names its vendor by id and the list follows the catalog.
 */
export class VendorCatalog {
  constructor(private readonly options: { fetch?: ModelsDevFetch; url?: string; now?: () => Date } = {}) {}

  async list(): Promise<Vendor[]> {
    const { catalog } = await fetchModelsDev(this.options)
    return Object.values(catalog)
      .map(vendorOf)
      .filter((vendor): vendor is Vendor => vendor !== null)
      .sort((a, b) => a.name.localeCompare(b.name))
  }

  async get(vendorId: string): Promise<Vendor | null> {
    const { catalog } = await fetchModelsDev(this.options)
    const entry = catalog[vendorId]
    return entry ? vendorOf(entry) : null
  }

  /** The vendor's model list as the catalog of provider entry `providerId`, or null for an unknown vendor. */
  async models(vendorId: string, providerId: string): Promise<ProviderModelList | null> {
    const snapshot = await fetchModelsDev(this.options)
    if (!snapshot.catalog[vendorId] || !vendorOf(snapshot.catalog[vendorId])) return null
    return modelListFromModelsDev(snapshot, vendorId, providerId)
  }
}

function vendorOf(entry: ModelsDevProvider): Vendor | null {
  if (NOT_OFFERED.has(entry.id)) return null
  const family = entry.npm ? FAMILY_BY_NPM[entry.npm] : undefined
  if (!family) return null
  return { id: entry.id, name: entry.name, ...family, baseUrl: entry.api ?? null, doc: entry.doc ?? null }
}
