import { inject, provide, type InjectionKey } from 'vue'
import type { ShellEditsView } from '@demicodes/agent/client'

export type EditSelectionHandler = (call: ShellEditsView, path: string) => void
const editSelectionKey: InjectionKey<EditSelectionHandler> = Symbol('edit-selection')

export function provideEditSelection(handler: EditSelectionHandler): void {
  provide(editSelectionKey, handler)
}

export function useEditSelection(): EditSelectionHandler | undefined {
  return inject(editSelectionKey, undefined)
}
