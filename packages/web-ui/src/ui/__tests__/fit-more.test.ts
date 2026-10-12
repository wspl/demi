import { expect, test } from 'bun:test'
import { fitMore, type FitMore } from '../fit-more'

// "Fit N, count the rest as +N": a row's pictures, a line of tags and the
// changed files' pills share this arithmetic on widths. It costs nothing.

/** The count drawn 20 px wide, whatever it says, so the cases read in round numbers. */
const count = () => 20

const cases: { name: string; room: number; widths: number[]; gap?: number; rows?: number; fit: FitMore }[] = [
  { name: 'everything that fits shows, with no count', room: 100, widths: [30, 30], fit: { shown: 2, more: 0 } },
  { name: 'an exact fit shows everything', room: 64, widths: [30, 30], fit: { shown: 2, more: 0 } },
  // Four items need 132; two and the count take 30 + 4 + 30 + 4 + 20 = 88, three and the count 122.
  { name: 'the rest are counted, as many shown as leave room for the count', room: 100, widths: [30, 30, 30, 30], fit: { shown: 2, more: 2 } },
  { name: 'a line that has room for the count alone counts every item', room: 30, widths: [40, 40], fit: { shown: 0, more: 2 } },
  { name: 'a line too narrow for even the count shows and counts nothing', room: 10, widths: [40, 40], fit: { shown: 0, more: 0 } },
  { name: 'no items hold nothing', room: 100, widths: [], fit: { shown: 0, more: 0 } },
  { name: 'an item wider than the line fits nowhere', room: 50, widths: [60], fit: { shown: 0, more: 1 } },
  // Two lines of 90 hold two items each.
  { name: 'items flow onto the next line within the rows allowed', room: 90, widths: [40, 40, 40, 40], rows: 2, fit: { shown: 4, more: 0 } },
  // Three items and the count make [40, 40] and [40, count].
  { name: 'the count lands on the last allowed line', room: 90, widths: [40, 40, 40, 40, 40, 40], rows: 2, fit: { shown: 3, more: 3 } },
  { name: 'a count that would start a new line past the limit costs an item', room: 90, widths: [40, 40, 40, 40, 40], rows: 2, fit: { shown: 3, more: 2 } },
]

for (const { name, room, widths, gap = 4, rows, fit } of cases) {
  test(name, () => {
    expect(fitMore({ room, widths, gap, rows, countWidth: count })).toEqual(fit)
  })
}

test('a count of two digits takes more room than one of one digit', () => {
  const countWidth = (rest: number) => (rest >= 10 ? 40 : 30)
  // Four 20 px items and "+8" take 92 + 4 + 30 = 126 of 128; with "+10" they would take 136.
  expect(fitMore({ room: 128, widths: Array(12).fill(20), gap: 4, countWidth })).toEqual({ shown: 4, more: 8 })
  expect(fitMore({ room: 128, widths: Array(14).fill(20), gap: 4, countWidth })).toEqual({ shown: 3, more: 11 })
})
