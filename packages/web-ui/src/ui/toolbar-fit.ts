/**
 * How many of a toolbar's actions, from its start, show in `room` pixels,
 * each `widths[i]` wide with what precedes it in its group, `gap` apart;
 * the rest go into the More menu, `more` wide, at the end. All show when
 * all fit; otherwise as many as fit beside More, at least none.
 */
export function fittingActions(widths: readonly number[], gap: number, room: number, more: number): number {
  const span = (count: number) => {
    let total = 0
    for (let index = 0; index < count; index++) {
      total += widths[index]! + (index > 0 ? gap : 0)
    }
    return total
  }
  if (span(widths.length) <= room) {
    return widths.length
  }
  let count = widths.length - 1
  while (count > 0 && span(count) + gap + more > room) {
    count--
  }
  return count
}
