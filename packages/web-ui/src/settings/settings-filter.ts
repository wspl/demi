import type { SettingsEntry, SettingsNavGroup, SettingsNavItem } from './types'

/** A section the rail filter found, with the settings of it that match. */
export interface SettingsMatch {
  item: SettingsNavItem
  /** The section's settings whose label or keywords match, in page order. */
  settings: SettingsEntry[]
}

/** A group of the rail as the filter leaves it. */
export interface SettingsMatchGroup {
  label?: string
  matches: SettingsMatch[]
}

function hits(words: readonly string[], query: string): boolean {
  return words.some((word) => word.toLowerCase().includes(query))
}

/**
 * The rail for `query`: every section whose name or keywords match, or that
 * holds a setting that does, each with its matching settings; groups left
 * empty go. An empty query leaves every section and lists no setting.
 */
export function filterSettings(groups: readonly SettingsNavGroup[], query: string): SettingsMatchGroup[] {
  const q = query.trim().toLowerCase()
  return groups
    .map((group) => ({
      label: group.label,
      matches: group.items.flatMap((item): SettingsMatch[] => {
        if (!q) {
          return [{ item, settings: [] }]
        }
        const settings = (item.settings ?? []).filter((entry) => hits([entry.label, ...(entry.keywords ?? [])], q))
        const named = hits([item.label, ...(item.keywords ?? [])], q)
        return named || settings.length ? [{ item, settings }] : []
      }),
    }))
    .filter((group) => group.matches.length)
}

/**
 * What Enter in the filter opens: the first setting found in an enabled
 * section, or else the first enabled section; null when nothing can open.
 */
export function firstMatch(groups: readonly SettingsMatchGroup[]): { section: string; setting: string | null } | null {
  const open = groups.flatMap((group) => group.matches).filter((match) => !match.item.disabled)
  const withSetting = open.find((match) => match.settings.length)
  if (withSetting) {
    return { section: withSetting.item.id, setting: withSetting.settings[0]!.label }
  }
  return open[0] ? { section: open[0].item.id, setting: null } : null
}
