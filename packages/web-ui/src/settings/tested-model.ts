import type { SettingsProviderEntry, SettingsProviderModel } from './types'

/**
 * The model Test Connection sends its short chat message to: the first
 * text model the user keeps enabled; none when the entry has none, such as
 * an entry that only makes images, whose models cost money to try
 * (`web-api-settings.md` § Providers).
 */
export function testedModel(provider: SettingsProviderEntry): SettingsProviderModel | undefined {
  return provider.models.find((model) => (model.kind ?? 'text') === 'text' && model.enabled)
}

/** Whether the entry has a text model at all, enabled or not. */
export function hasTextModel(provider: SettingsProviderEntry): boolean {
  return provider.models.some((model) => (model.kind ?? 'text') === 'text')
}
