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

export const THINKING_EFFORTS = ['minimal', 'low', 'medium', 'high', 'max'] as const

export const EXTENSION_PRESETS: { id: string; label: string; extensions: string[] }[] = [
  { id: 'images', label: 'Images', extensions: ['.png', '.jpg', '.jpeg', '.gif', '.webp'] },
  { id: 'videos', label: 'Videos', extensions: ['.mp4', '.mov', '.webm'] },
  { id: 'documents', label: 'Documents', extensions: ['.pdf', '.txt', '.md', '.docx', '.csv'] },
]
