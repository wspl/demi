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
