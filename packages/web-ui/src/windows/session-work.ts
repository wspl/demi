import { shallowRef, watch, type ShallowRef } from 'vue'
import type { PanelState, PinnedTabs } from '../agent/panel-tabs'
import type { PanelTabKind } from '../agent/panel-kinds/kind'
import type { TabUpdate } from '../plugins/page'
import { useWindowStage, type StageState, type WindowStageModel } from './stage'

/**
 * What the host gives a conversation's windows: its tabs and their kinds, the
 * page's address over them, the layout the conversation keeps, and the user's
 * pins. The product's comes from the work store (`web/conversation/work.ts`),
 * the gallery's from its fixture.
 */
export interface SessionWork {
  /** The tabs, in order, and what the address selected, the newest last. */
  panel: PanelState
  pinned: PinnedTabs
  /** The kinds of the pages the user has on, bound to the conversation. */
  kinds: readonly PanelTabKind[]
  /** Whether the host has read the tabs: the ones a first read brings come with no motion. */
  tabsRead: boolean
  /** The conversation's first send has not created it yet: Files and Changes say when they come. */
  beforeFirstMessage: boolean
  /** The name of the primary device while its runner is not connected, which a window says over its tabs. */
  deviceOffline: string | null
  /** Whether the address names a tab, which the window holding it shows in front. */
  open: boolean
  /**
   * The address names the tab `id`: its window comes in front on it. A step
   * of the history, or with `replace` the address in place of the one shown.
   */
  select(id: string, how?: 'push' | 'replace'): void
  /** The address names no tab: no window of the work is in front. */
  close(): void
  moveTab(id: string, index: number): void
  updateTab(id: string, data: unknown, how?: TabUpdate): void
  updatePinned(kind: string, data: unknown, how?: TabUpdate): void
  closeTabs(ids: string[]): void
  /** A new tab of `kind` with `data`, selected. */
  add(kind: string, data: unknown): void
  /** The layout the conversation keeps, null while it has none. */
  layout: StageState | null
  /** Keeps the layout with the conversation. */
  keepLayout(layout: StageState): void
  /** The windows the user pinned to the taskbar, for every conversation, in order. */
  pins: readonly string[]
  /** Keeps `pins` as the user's pins, in their order. */
  keepPins(pins: readonly string[]): void
}

/**
 * The stage of the conversation `conversation()` names: made from the layout
 * it keeps when it comes on screen, and kept with it at each change that ends
 * (`settled`). The layout is read once per showing: one kept by another
 * device while this one shows the conversation shows the next time, and one
 * that comes after the conversation does replaces the stage only while
 * nothing moved in it. A page that does not write the layout, a phone's,
 * opens the conversation with every window put away.
 */
export function useSessionStage(
  conversation: () => string,
  work: () => SessionWork | undefined,
  writes: () => boolean,
): ShallowRef<WindowStageModel> {
  const made = (layout: StageState | undefined) => useWindowStage(layout, { putAway: !writes() })
  const stage = shallowRef(made(work()?.layout ?? undefined))
  watch(conversation, () => {
    stage.value = made(work()?.layout ?? undefined)
  })
  watch(() => work()?.layout ?? null, (layout, before) => {
    if (layout && !before && stage.value.settled.value === 0) {
      stage.value = made(layout)
    }
  })
  // A stage made anew holds what was kept: only a change in it is written, and
  // an operation that moved nothing, such as a press in the window in front,
  // writes nothing.
  let kept = JSON.stringify(stage.value.state)
  watch([stage, () => stage.value.settled.value], ([now], [before]) => {
    const layout = JSON.stringify(now.state)
    const host = work()
    if (now === before && layout !== kept && host && writes()) {
      host.keepLayout(JSON.parse(layout) as StageState)
    }
    kept = layout
  })
  return stage
}
