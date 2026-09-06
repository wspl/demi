/** Presentation models for the settings surfaces. Hosts map their own state onto these. */

export type SettingsTab = 'Account' | 'Devices' | 'Providers' | 'Usage'

export const SETTINGS_TABS: readonly SettingsTab[] = ['Account', 'Devices', 'Providers', 'Usage']

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
