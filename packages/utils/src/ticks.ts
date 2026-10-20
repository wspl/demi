/**
 * The round step for about `count` ticks over `span`: 1, 2 or 5 times a power
 * of ten, whichever lies nearest to `span / count` on a logarithmic scale, as
 * chart libraries choose theirs.
 */
export function niceStep(span: number, count: number): number {
  const raw = span / Math.max(1, count)
  if (!(raw > 0))
    return 1
  const power = 10 ** Math.floor(Math.log10(raw))
  const ratio = raw / power
  const factor = ratio >= Math.sqrt(50) ? 10 : ratio >= Math.sqrt(10) ? 5 : ratio >= Math.sqrt(2) ? 2 : 1
  return factor * power
}

/**
 * The multiples of a round step within `[from, to]`, about `count` of them,
 * the step never below `minStep`.
 */
export function niceTicks(from: number, to: number, count: number, minStep = 0): number[] {
  const step = Math.max(minStep, niceStep(to - from, count))
  const ticks: number[] = []
  for (let index = Math.ceil(from / step); index * step <= to; index++)
    ticks.push(index * step)
  return ticks
}
