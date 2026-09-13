import { inject, provide, type InjectionKey } from 'vue'
import type { CallEditSelection } from '../files/changes'

export type EditSelectionHandler = (selection: CallEditSelection) => void
const editSelectionKey: InjectionKey<EditSelectionHandler> = Symbol('edit-selection')

export function provideEditSelection(handler: EditSelectionHandler): void {
  provide(editSelectionKey, handler)
}

export function useEditSelection(): EditSelectionHandler | undefined {
  return inject(editSelectionKey, undefined)
}
