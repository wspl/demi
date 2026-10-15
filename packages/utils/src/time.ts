/** The time left until `deadline`, at `now`, both on the page's clock, as a code's countdown says it: `9:05`; null once it passed. */
export function timeLeft(deadline: number, now: number): string | null {
  const seconds = Math.ceil((deadline - now) / 1000)
  if (seconds <= 0) {
    return null
  }
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
