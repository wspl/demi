/** Presentation models for the settings surfaces. Hosts map their own state onto these. */
import type { Component } from 'vue'

/** A section id. Hosts choose their own set; the built-in four cover the product today. */
export type SettingsTab = string

export interface SettingsNavItem {
  id: SettingsTab
  label: string
  icon: Component
}

/** Sections grouped under a small caption, the way a long rail is read. */
export interface SettingsNavGroup {
  label?: string
  items: SettingsNavItem[]
}

export interface SettingsAccountInfo {
  name: string
  plan: string
}

export interface SettingsDevice {
  id: string
  name: string
  online: boolean
}

export interface SettingsProvider {
  id: string
  label: string
  modelCount: number
  isAvailable: boolean
}

export interface SettingsUsage {
  conversations: number
  messages: number
  /** Formatted by the host; the panel shows it verbatim. */
  cost: string
}

/** A model as the settings edit it: what a custom endpoint needs to be usable. */
export interface SettingsModelDraft {
  id: string
  name: string
  contextWindow: number | null
  outputLimit: number | null
  /** Thinking efforts the composer offers; the first is the default. Empty means no control. */
  efforts: string[]
  /** Accepted attachment extensions, dot-prefixed. Empty means text only. */
  extensions: string[]
  fastTier: string | null
}

/** The API format an endpoint speaks. */
export type SettingsWireApi = 'anthropic-messages' | 'openai-responses' | 'openai-chat'

export const WIRE_API_LABELS: Record<SettingsWireApi, string> = {
  'anthropic-messages': 'Anthropic Messages',
  'openai-responses': 'OpenAI Responses',
  'openai-chat': 'OpenAI Chat Completions',
}

/** A models.dev vendor an API key can be added for. */
export interface SettingsVendor {
  id: string
  name: string
  wireApi: SettingsWireApi
  /** Null when the runtime knows the vendor's endpoint itself. */
  baseUrl: string | null
  logo: string
}

/** `unconfigured` is a fresh entry that still needs its key or login. */
export type SettingsProviderState = 'ready' | 'unconfigured' | 'error' | 'unreachable' | 'signed-out' | 'disabled'

/** A model a provider offers, as the page lists and toggles it. */
export interface SettingsProviderModel extends SettingsModelDraft {
  enabled: boolean
}

export interface SettingsQuotaWindow {
  used: number
  max: number
  resets: string
}

export interface SettingsProviderAccount {
  id: string
  label: string
  plan: string
  active: boolean
  /** Rate-window quotas, when the vendor exposes them. */
  quota: { hour: SettingsQuotaWindow; week: SettingsQuotaWindow } | null
}

/**
 * A provider as the settings page shows and edits it. The page edits fields in
 * place (name, endpoint, key, enabled, model toggles) and emits everything that
 * needs the host: adding, removing, signing in, testing, refreshing, saving a model.
 */
export interface SettingsProviderEntry {
  id: string
  name: string
  kind: 'api_key' | 'subscription'
  /** models.dev vendor; null for a bare endpoint. */
  vendorId: string | null
  baseUrl: string
  wireApi: SettingsWireApi
  apiKey: string
  /** Where the model list comes from: the vendor catalog, or ids the user typed. */
  modelSource: 'catalog' | 'manual'
  catalogFetched: string | null
  stale: boolean
  state: SettingsProviderState
  /** What went wrong, from the last test or request. */
  detail?: string
  /** How long the last successful test took, formatted by the host. */
  testedIn?: string
  enabled: boolean
  models: SettingsProviderModel[]
  accounts: SettingsProviderAccount[]
  /** The vendor mark; null falls back to an initial. */
  logo: string | null
}

export const THINKING_EFFORTS = ['minimal', 'low', 'medium', 'high', 'max'] as const

export const EXTENSION_PRESETS: { id: string; label: string; extensions: string[] }[] = [
  { id: 'images', label: 'Images', extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp'] },
  { id: 'videos', label: 'Videos', extensions: ['.mp4', '.mov', '.webm'] },
  { id: 'documents', label: 'Documents', extensions: ['.pdf', '.txt', '.md', '.docx', '.csv'] },
]
