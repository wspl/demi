import type { Component } from 'vue'
import type { PanelTabKind } from '../agent/panel-kinds/kind'
import type { SettingsNavGroup, SettingsNavItem } from '../settings/types'
import { conversationClient, type ConversationPluginClient, type PluginHost } from './client'

/**
 * What a plugin package fills of the web app (`plugins.md` § The page):
 * `web` and `web-gallery` register each package's `PluginPage` from a static
 * list, and the slots below show what the pages fill.
 */
export interface PluginPage {
  /** The plugin's id, whose state and calls the slots reach. */
  plugin: string
  /**
   * The page methods and user streams the slots use: while the backend
   * serves one of them not, as when its catalog lacks the package they bind,
   * the page fills no slot.
   */
  uses?: { methods?: readonly string[]; streams?: readonly string[] }
  /** A settings section: its navigation entry, in a group of the rail, and its page. */
  settings?: PluginSettingsSection
  /** Work panel kinds, made for each conversation whose panel is open. */
  panelKinds?: (context: PanelKindsContext) => PanelKinds
  /**
   * A tool in the conversation header, which shows itself only while the
   * plugin's state has something to show. It receives `PluginHeaderToolProps`
   * and emits `openTab(kind, data)` and `manageDevices()`.
   */
  headerTool?: Component
}

export interface PluginSettingsSection {
  /** The label of the rail's group it joins. */
  group: string
  item: SettingsNavItem
  /** The page, which receives `overlayStore`. */
  component: Component
}

/** What a conversation's work panel offers a plugin's kinds. */
export interface PanelKindsContext {
  conversation: string
  plugin: ConversationPluginClient
  tabs: {
    /** The `data` of the panel's tabs of `kind`, as saved. */
    bound(kind: string): unknown[]
    /** Adds a tab of `kind` without selecting it. */
    add(kind: string, data: unknown): void
  }
}

/** A plugin's kinds for one conversation's panel. */
export interface PanelKinds {
  kinds: PanelTabKind[]
  /** The conversation's agent may have changed what the kinds show, as after a tool call. */
  refresh?(): void
  /** The panel closed or shows another conversation. */
  dispose?(): void
}

export interface PluginHeaderToolProps {
  conversationId: string
  /** The name of the user's device `id`, as the page shows hosts. */
  hostName: (id: string) => string
}

/** A plugin as the product state lists it, as far as its page's slots ask. */
export interface ListedPlugin {
  id: string
  enabled: boolean
  methods: readonly string[]
  streams: readonly string[]
}

/**
 * Whether `page` fills its slots: its plugin is listed and on, and serves
 * every method and stream the page uses.
 */
export function pluginPageShown(page: PluginPage, plugins: readonly ListedPlugin[]): boolean {
  const plugin = plugins.find((candidate) => candidate.id === page.plugin)
  if (!plugin?.enabled) {
    return false
  }
  const methods = page.uses?.methods ?? []
  const streams = page.uses?.streams ?? []
  return (
    methods.every((method) => plugin.methods.includes(method)) &&
    streams.every((stream) => plugin.streams.includes(stream))
  )
}

/**
 * The settings rail with each shown page's section joined to its group, at
 * the group's end; a group the rail lacks is added at the end.
 */
export function withPluginSections(
  groups: readonly SettingsNavGroup[],
  pages: readonly PluginPage[],
  shown: (page: PluginPage) => boolean,
): SettingsNavGroup[] {
  const joined = groups.map((group) => ({ ...group, items: [...group.items] }))
  for (const page of pages) {
    if (!page.settings || !shown(page)) {
      continue
    }
    const group = joined.find((candidate) => candidate.label === page.settings!.group)
    if (group) {
      group.items.push(page.settings.item)
    } else {
      joined.push({ label: page.settings.group, items: [page.settings.item] })
    }
  }
  return joined
}

/** The page of the settings section `tab`, if a plugin fills it. */
export function pluginSettingsPage(pages: readonly PluginPage[], tab: string): Component | null {
  return pages.find((page) => page.settings?.item.id === tab)?.settings?.component ?? null
}

/** Every shown page's kinds for one conversation's panel, refreshed and disposed together. */
export function pluginPanelKinds(
  pages: readonly PluginPage[],
  host: PluginHost,
  shown: (page: PluginPage) => boolean,
  context: Omit<PanelKindsContext, 'plugin'>,
): Required<PanelKinds> {
  const made = pages
    .filter((page) => page.panelKinds && shown(page))
    .map((page) =>
      page.panelKinds!({
        ...context,
        plugin: conversationClient(host, page.plugin, context.conversation),
      }),
    )
  return {
    kinds: made.flatMap((kinds) => kinds.kinds),
    refresh: () => made.forEach((kinds) => kinds.refresh?.()),
    dispose: () => made.forEach((kinds) => kinds.dispose?.()),
  }
}
