import { shallowRef } from 'vue'
import type { SelectionFieldActions } from '../../ui/context-menu'
import { t } from '../locales/text'

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

/** What stands for an image or other media in a quote of the selection. */
const MEDIA = 'img, picture, video, audio, canvas, svg, iframe, object, embed'

/**
 * The selected text as the phone's field quotes it, on one line: each image or
 * other media in it said as the media text. An icon the page hides from
 * assistive technology, as a link's, is decoration and says nothing.
 */
export function quoteOfRange(range: Range): string {
  const holder = document.createElement('div')
  holder.append(range.cloneContents())
  for (const media of holder.querySelectorAll(MEDIA)) {
    if (media.closest('[aria-hidden="true"]'))
      media.remove()
    else
      media.replaceWith(` ${t('comments.media')} `)
  }
  return (holder.textContent ?? '').replace(/\s+/g, ' ').trim()
}
