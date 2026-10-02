import { inject, provide, type InjectionKey } from 'vue'
import type { CallEditSelection } from '../files/changes'

/** Opens one call's edit of one file, through the `edit` intent (`plugin-pages.md` § Intents). */
export type EditSelectionHandler = (selection: CallEditSelection) => void
const editSelectionKey: InjectionKey<() => EditSelectionHandler | undefined> = Symbol('edit-selection')

/**
 * Where the transcript below sends a picked file pill, read at each render:
 * none while no plugin the user has on opens the edit, and the pills are then
 * no controls.
 */
export function provideEditSelection(handler: () => EditSelectionHandler | undefined): void {
  provide(editSelectionKey, handler)
}

export function useEditSelection(): () => EditSelectionHandler | undefined {
  return inject(editSelectionKey, () => undefined)
}
