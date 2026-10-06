import { z } from 'zod'
import { TAB_HISTORY } from '@demicodes/web-ui/agent/tab-close'

const localStateSchema = z.object({
  hiddenProviders: z.array(z.string()),
  hiddenModels: z.record(z.string(), z.array(z.string())),
  recentProjects: z.array(z.string()),
  foldedProjects: z.array(z.string()),
  /** The sidebar's width in px; absent until the reader resizes it. */
  sidebarWidth: z.number().int().optional(),
  /** The work panel's share of the width it splits with the conversation. */
  asideShare: z.number().min(0).max(1).optional(),
  /** Whether the work panel is open, keyed by conversation id. */
  workPanelOpen: z.record(z.string(), z.boolean()).optional(),
  /** What this page's work panel selected, tabs' ids and pinned kinds', the newest last, keyed by conversation id. */
  workPanelHistory: z.record(z.string(), z.array(z.string()).max(TAB_HISTORY)).optional(),
  /**
   * Each tab's highest request to be shown that this page's work panel
   * applied, by tab id, keyed by conversation id (`live-view.md` § Showing a tab).
   */
  workPanelShown: z.record(z.string(), z.record(z.string(), z.number().int().min(1))).optional(),
})
export type LocalState = z.infer<typeof localStateSchema>

export function emptyLocalState(): LocalState {
  return {
    hiddenProviders: [],
    hiddenModels: {},
    recentProjects: [],
    foldedProjects: [],
  }
}

export function readLocalState(userId: string): LocalState {
  try {
    const raw = localStorage.getItem(`demi.preferences.${userId}`)
    return raw ? localStateSchema.parse(JSON.parse(raw)) : emptyLocalState()
  } catch (error) {
    // The web browser's storage is optional. Invalid external entries are
    // discarded as a whole; no legacy keys or partially repaired records enter
    // product state.
    console.warn('Could not read local preferences', error)
    return emptyLocalState()
  }
}

export function writeLocalState(userId: string, value: LocalState): void {
  try {
    localStorage.setItem(`demi.preferences.${userId}`, JSON.stringify(value))
  } catch (error) {
    // Private browsing and storage quotas must not prevent server operations.
    console.warn('Could not save local preferences', error)
  }
}
