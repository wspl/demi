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
  titles: boolean
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
  'titles',
]

/** A Cloud's CPUs or memory: an amount, or all the server has. */
export type CloudLimit = number | 'unlimited'

/**
 * How every Cloud of an integration is sized, which only an administrator
 * changes; null is the server's: the Cloud manager's CPUs and memory, the
 * account's disks, 16 GB for the shared folder.
 */
export interface CloudSizing {
  cpus: CloudLimit | null
  memoryMib: CloudLimit | null
  systemGb: number | null
  homeGb: number | null
  sharedGb: number | null
}

/**
 * Each size an integration's Cloud gets when its integration leaves the size to the
 * server, and the server's CPUs and memory, which a Cloud without a limit may use
 * (`crates/backend-user-shard/src/integrations.rs`); CPUs and memory are null until the
 * Cloud manager has said them.
 */
export interface ServerSizes {
  cpus: number | null
  memoryMib: number | null
  systemGb: number
  homeGb: number
  sharedGb: number
  idleMinutes: number
  serverCpus: number | null
  serverMemoryMib: number | null
  /** Whether the chosen isolation enforces a Cloud's CPUs, memory and disks. */
  limits: boolean
}

/** The sizes a Cloud may have without a limit, in the order the page lists them. */
export const CLOUD_LIMITS = ['cpus', 'memoryMib'] as const satisfies readonly (keyof CloudSizing)[]

/** A Cloud's disks, in the order the page lists them. */
export const CLOUD_DISKS = ['systemGb', 'homeGb'] as const satisfies readonly (keyof CloudSizing)[]

/** How a limit is chosen: the server's, none, or an amount. */
export type LimitChoice = 'default' | 'unlimited' | 'custom'

export const LIMIT_CHOICES: readonly LimitChoice[] = ['default', 'unlimited', 'custom']

export function limitChoice(limit: CloudLimit | null): LimitChoice {
  return limit === null ? 'default' : limit === 'unlimited' ? 'unlimited' : 'custom'
}

/** The amount a Cloud gets of `size` as `limit` sets it; null when the server hasn't said. */
export function limitAmount(size: (typeof CLOUD_LIMITS)[number], limit: CloudLimit | null, server: ServerSizes | null): number | null {
  if (typeof limit === 'number')
    return limit
  if (!server)
    return null
  if (limit === 'unlimited')
    return size === 'cpus' ? server.serverCpus : server.serverMemoryMib
  return server[size]
}

/** The unit a size is in, as its message's key; a count of CPUs has none. */
export const SIZE_UNITS: Readonly<Record<keyof CloudSizing, 'memoryMibUnit' | 'gbUnit' | null>> = {
  cpus: null,
  memoryMib: 'memoryMibUnit',
  systemGb: 'gbUnit',
  homeGb: 'gbUnit',
  sharedGb: 'gbUnit',
}

/** A whole number of at least 1 as a field holds it; null for an empty field, NaN for anything else. */
export function parseSize(text: string): number | null {
  const trimmed = text.trim()
  if (!trimmed)
    return null
  const size = /^\d+$/.test(trimmed) ? Number(trimmed) : Number.NaN
  return size >= 1 && Number.isSafeInteger(size) ? size : Number.NaN
}

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
  cloudSizing: CloudSizing
  /** How long each of its Clouds runs without activity before it stops, in minutes; null for the server's hour. */
  cloudIdleMinutes: number | null
  clouds: number
  conversations: number
}

/** A change of an integration; a field left out stays. */
export interface IntegrationChange {
  name?: string
  capabilities?: IntegrationCapabilities
  initScript?: string | null
  sharedPath?: string
  /** Whole: only an administrator's is taken. */
  cloudSizing?: CloudSizing
  cloudIdleMinutes?: number | null
}

/** What an integration's row says under its name: *2 Clouds · 14 conversations*. */
export function integrationSummary(integration: Pick<SettingsIntegration, 'clouds' | 'conversations'>): string {
  return [
    t('integrations.clouds', { count: integration.clouds }, integration.clouds),
    t('integrations.conversations', { count: integration.conversations }, integration.conversations),
  ].join(' · ')
}
