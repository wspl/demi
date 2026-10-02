import type { Component } from 'vue'
import type { PanelTabKind } from '../agent/panel-kinds/kind'
import type { SettingsNavGroup, SettingsNavItem } from '../settings/types'
import type { ChangeSetSource, ReadCallChange } from '../files/changes'
import type { FileBrowserSource } from '../files/types'
import { conversationClient, type ConversationPluginClient, type PluginHost } from './client'
import type { IntentName, IntentPayloads, IntentTarget, PageIntents } from './intents'

/**
 * What a plugin package fills of the web app (`plugin-pages.md` § Slots):
 * `web` and `web-gallery` register each package's `PluginPage` from a static
 * list, and the slots below show what the pages fill.
 */
export interface PluginPage {
  /** The plugin's id, whose state and calls the slots reach. */
  plugin: string
  /** A settings section: its navigation entry, in a group of the rail, and its page. */
  settings?: PluginSettingsSection
  /** Work panel kinds, made for each conversation whose panel is open. */
  panelKinds?: (context: PanelKindsContext) => PanelKinds
  /**
   * The intents its pinned kinds open. They are declared here rather than on
   * a kind because an intent can arrive while the panel is closed and no
   * kind is made.
   */
  intents?: PageIntents
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

/**
 * A conversation's files as the work panel's kinds read them
 * (`plugin-pages.md` § Services): a product service, whoever shows them.
 */
export interface ConversationFileService {
  /** The Host's file tree and file contents, while the conversation's Host is known. */
  workspace: { source: FileBrowserSource; root: string; name?: string } | null
  /** Where the conversation's work runs, which a retained edit's paths are relative to. */
  root: string | null
  /** The working tree's uncommitted changes. */
  changes: ChangeSetSource
  /** The two sides of one call's retained edit. */
  readCallChange: ReadCallChange
}

/** Opening intents for one conversation (`plugin-pages.md` § Intents). */
export interface IntentService {
  open<Name extends IntentName>(intent: Name, payload: IntentPayloads[Name]): void
  /** Whether any page the user has on opens `intent`; a control that would open it shows only then. */
  canOpen(intent: IntentName): boolean
}

/** What a conversation's work panel offers a plugin's kinds. */
export interface PanelKindsContext {
  conversation: string
  plugin: ConversationPluginClient
  files: ConversationFileService
  intents: IntentService
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

/**
 * The settings rail with each enabled page's section joined to its group,
 * at the group's end; a group the rail lacks is added at the end.
 */
export function withPluginSections(
  groups: readonly SettingsNavGroup[],
  pages: readonly PluginPage[],
  enabled: (plugin: string) => boolean,
): SettingsNavGroup[] {
  const joined = groups.map((group) => ({ ...group, items: [...group.items] }))
  for (const page of pages) {
    if (!page.settings || !enabled(page.plugin)) {
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

/** Every enabled page's kinds for one conversation's panel, refreshed and disposed together. */
export function pluginPanelKinds(
  pages: readonly PluginPage[],
  host: PluginHost,
  enabled: (plugin: string) => boolean,
  context: Omit<PanelKindsContext, 'plugin'>,
): Required<PanelKinds> {
  const made = pages
    .filter((page) => page.panelKinds && enabled(page.plugin))
    .map((page) =>
      page.panelKinds!({
        ...context,
        plugin: conversationClient(host, page.plugin, context.conversation),
      }),
    )
  const kinds = made.flatMap((each) => each.kinds)
  const ids = kinds.map((kind) => kind.kind)
  const repeated = ids.find((id, index) => ids.indexOf(id) !== index)
  if (repeated !== undefined) {
    throw new Error(`two plugin pages register the work panel kind ${repeated}`)
  }
  return {
    kinds,
    refresh: () => made.forEach((each) => each.refresh?.()),
    dispose: () => made.forEach((kinds) => kinds.dispose?.()),
  }
}

/** The first enabled page's target for `intent`, or null when no page the user has on opens it. */
export function intentTarget<Name extends IntentName>(
  pages: readonly PluginPage[],
  enabled: (plugin: string) => boolean,
  intent: Name,
): IntentTarget<Name> | null {
  for (const page of pages) {
    const target = page.intents?.[intent]
    if (target && enabled(page.plugin)) {
      return target
    }
  }
  return null
}
