import { fitMore, type FitMore } from '../ui/fit-more'
import { declaredSize, ROW_PICTURE, thumbnailBox, type PixelSize } from '../files/thumbnail'
import type { ToolPicture } from './tool-media'

/**
 * How a row's pictures share its one line with its face: they stand at the
 * row's end, right-aligned and at least 8 pixels after the face's last part,
 * the chevron or a tag, 4 pixels apart, and never wrap. The face gives way
 * first: its title truncates, but only down to half the row; then the pictures
 * that no longer fit are counted after the last one shown, `+5`. /
 */

/**
 * The box a row's picture takes, from the size its reference carries, known
 * before its bytes arrive: the picture and the row's fit read the same one.
 */
export function rowPictureBox(picture: ToolPicture): PixelSize {
  return thumbnailBox(declaredSize(picture.source), picture.kind, ROW_PICTURE)
}

/**
 * The least space between the face's last part and the first picture, the face
 * row's gap.
 */
export const ROW_PICTURES_LEAD = 8
/**
 * The space between two pictures, and between the last one shown and the count
 * of the rest.
 */
export const ROW_PICTURES_GAP = 4

/**
 * Which of the pictures of `widths`, in their order, show in a row `row`
 * pixels wide whose face is `face` pixels wide when nothing cuts it, given
 * how wide the count of `n` pictures is drawn (`moreWidth`). The pictures
 * take what the face leaves, the face keeping its own width or half the
 * row, whichever is less; when they do not all fit, as many show as leave
 * room for the count of the rest. In a row too narrow for even the count,
 * nothing shows.
 */
export function fitRowPictures(
  row: number,
  face: number,
  widths: readonly number[],
  moreWidth: (count: number) => number,
): FitMore {
  const room = row - ROW_PICTURES_LEAD - Math.min(face, row / 2)
  return fitMore({ room, widths, gap: ROW_PICTURES_GAP, countWidth: moreWidth })
}
