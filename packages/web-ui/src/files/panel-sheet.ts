import { inject, type InjectionKey, type Ref } from 'vue'

/**
 * How a view inside a phone's work-panel overlay closes it (`WorkPanel`'s
 * `sheet`): the overlay provides what closes it to the conversation, which a
 * view calls when the user asks to leave it: its Close button
 * (`SheetCloseButton`), a tap beside a picture or a video, or a pull down on
 * one. Null while the view is not in an overlay, as beside the conversation on
 * a computer or in the full panel.
 */
export const sheetDismissKey: InjectionKey<Ref<(() => void) | null>> = Symbol('sheetDismiss')

/** What closes the overlay a view stands in, or null while it stands in none. */
export function useSheetDismiss(): Ref<(() => void) | null> | null {
  return inject(sheetDismissKey, null)
}
