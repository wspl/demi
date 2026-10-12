import { expect, test } from 'bun:test'
import { fitRowPictures } from '../row-pictures'

// A row's pictures share its line with its face (`file-previews.md` § Media
// a tool returned): at the row's end, at least 8 px after the face, 4 px apart, the title giving way
// only down to half the row, then the rest counted as +N. Pure arithmetic
// on widths the row measured; it costs nothing.

/** The count drawn 20 px wide, whatever it says, so the cases read in round numbers. */
const more = () => 20
const shot = 36

const cases: { name: string; row: number; face: number; widths: number[]; fit: { shown: number; more: number } }[] = [
  { name: 'one screenshot after a short title shows', row: 600, face: 200, widths: [shot], fit: { shown: 1, more: 0 } },
  { name: 'four screenshots fit beside a short title', row: 600, face: 200, widths: [shot, shot, shot, shot], fit: { shown: 4, more: 0 } },
  // 600 - 8 - 200 leaves 392: nine screenshots take 9 × 36 + 8 × 4 = 356,
  // and a tenth would leave no room for the count of the last two.
  { name: 'twelve screenshots show as many as leave room for +N', row: 600, face: 200, widths: Array(12).fill(shot), fit: { shown: 9, more: 3 } },
  // The face is 500 wide, more than half the row: it keeps 300 and the
  // pictures get 600 - 8 - 300 = 292, six screenshots and the count.
  { name: 'a long title truncates down to half the row and no further', row: 600, face: 500, widths: Array(12).fill(shot), fit: { shown: 6, more: 6 } },
  // Four screenshots take 156: a 500-wide face gives way to 436, above half the row.
  { name: 'a long title gives way to pictures that all fit beside half the row', row: 600, face: 500, widths: [shot, shot, shot, shot], fit: { shown: 4, more: 0 } },
  { name: 'a row too narrow for one picture counts them all', row: 80, face: 30, widths: [shot, shot], fit: { shown: 0, more: 2 } },
  { name: 'a row too narrow for the count shows nothing', row: 40, face: 30, widths: [shot, shot], fit: { shown: 0, more: 0 } },
  { name: 'a row without pictures holds none', row: 600, face: 200, widths: [], fit: { shown: 0, more: 0 } },
]

for (const { name, row, face, widths, fit } of cases) {
  test(name, () => {
    expect(fitRowPictures(row, face, widths, more)).toEqual(fit)
  })
}
