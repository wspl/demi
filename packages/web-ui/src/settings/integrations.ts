import { t } from './locales/text'

/** What an integration's agents may do (`crates/shared-types/src/integration.rs`). */
export interface IntegrationCapabilities {
  subagents: boolean
  browser: boolean
  web: boolean
  skills: boolean
  media: boolean
  mobile: boolean
  schedules: boolean
  attachments: boolean
}

export type IntegrationCapability = keyof IntegrationCapabilities

/** The capabilities in the order the page lists them. */
export const CAPABILITIES: readonly IntegrationCapability[] = [
  'subagents',
  'browser',
  'web',
  'skills',
  'media',
  'mobile',
  'schedules',
  'attachments',
]

/** An integration as Settings › Integrations shows and edits it. */
export interface SettingsIntegration {
  id: string
  name: string
  capabilities: IntegrationCapabilities
  createdAt: string
  /** The shell script each of its Clouds runs once its system is new; null for none. */
  initScript: string | null
  /** Where each of its Clouds mounts its shared folder: `~/...` or a top-level directory. */
  sharedPath: string
  clouds: number
  conversations: number
}

/** A change of an integration; a field left out stays. */
export interface IntegrationChange {
  name?: string
  capabilities?: IntegrationCapabilities
  initScript?: string | null
  sharedPath?: string
}

/** What an integration's row says under its name: *2 Clouds · 14 conversations*. */
export function integrationSummary(integration: Pick<SettingsIntegration, 'clouds' | 'conversations'>): string {
  return [
    t('integrations.clouds', { count: integration.clouds }, integration.clouds),
    t('integrations.conversations', { count: integration.conversations }, integration.conversations),
  ].join(' · ')
}
