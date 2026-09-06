/** Presentation models for the settings surfaces. Hosts map their own state onto these. */
import type { Component } from 'vue'

export type SettingsTab = 'Account' | 'Devices' | 'Providers' | 'Usage'

export interface SettingsNavItem {
  id: SettingsTab
  label: string
  icon: Component
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
