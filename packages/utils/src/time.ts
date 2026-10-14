/**
 * A span of time in the interface's duration words, counted as a timer
 * counts: "42s", "2m 14s" (the seconds left out on the minute), "1h 5m"
 * (the minutes left out on the hour, the seconds never shown). Every unit is
 * floored, so a span reads as what has fully passed. The thinking label, the
 * command time tag, the agent status bar and the reset times all say their
 * spans with it.
 */
export function formatSpan(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000)
  if (seconds < 60) {
    return `${seconds}s`
  }
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) {
    return seconds % 60 ? `${minutes}m ${seconds % 60}s` : `${minutes}m`
  }
  const hours = Math.floor(minutes / 60)
  return minutes % 60 ? `${hours}h ${minutes % 60}m` : `${hours}h`
}

/** The time left until `deadline`, at `now`, both on the page's clock, as a code's countdown says it: `9:05`; null once it passed. */
export function timeLeft(deadline: number, now: number): string | null {
  const seconds = Math.ceil((deadline - now) / 1000)
  if (seconds <= 0) {
    return null
  }
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
