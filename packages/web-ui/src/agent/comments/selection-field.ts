import { shallowRef } from 'vue'
import type { SelectionFieldActions } from '../../ui/context-menu'

/**
 * What opens the comment field besides a selection, as a right-click menu's
 * rows do (`SelectionFieldHost` stands the one field by whatever is open).
 */

/** The action Return performs: the one the user chose. */
export type FieldWay = 'comment' | 'ask'

/** The picture a menu's Comment or Ask opened the field on, with what the field does for it. */
export interface PictureField {
  image: HTMLImageElement
  actions: SelectionFieldActions
  way: FieldWay
}

export const pictureField = shallowRef<PictureField | null>(null)

/**
 * The way a menu's Comment or Ask chose for the field the selection has. An
 * object each time, so the same choice twice is seen twice.
 */
export const chosenWay = shallowRef<{ way: FieldWay } | null>(null)

export function chooseWay(way: FieldWay): void {
  chosenWay.value = { way }
}

export function openPictureField(field: PictureField): void {
  pictureField.value = field
}

export function closePictureField(): void {
  pictureField.value = null
}
