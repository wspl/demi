// A wait with a bound, for a step that waits on something that may never
// answer, such as a script or a browser that hangs.

/** What `withinLimit` answers when the limit came first. */
export const OVERRAN = Symbol('overran')

/** Runs until `work` settles or `ms` pass, whichever comes first. */
export async function withinLimit<T>(work: Promise<T>, ms: number): Promise<T | typeof OVERRAN> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const limit = new Promise<typeof OVERRAN>((resolve) => {
    timer = setTimeout(() => resolve(OVERRAN), ms)
  })
  // Once the limit came first, the work's failure has no one to report to:
  // the caller already said that the work did not end in time.
  work.catch(() => undefined)
  try {
    return await Promise.race([work, limit])
  } finally {
    clearTimeout(timer)
  }
}

/** The longest delay a timer holds; a longer one fires at once. */
const LONGEST_TIMER_MS = 2 ** 31 - 1

/**
 * `--limit` (browse.md § A call) in milliseconds: seconds, or minutes with
 * an `m`. Throws, saying why, for anything else, and for a limit longer
 * than a timer holds, which would end the call as it starts.
 */
export function parseCallLimit(text: string): number {
  const limit = /^(\d+(?:\.\d+)?)(s|m)?$/.exec(text)
  if (!limit) {
    throw new Error(`--limit takes seconds or minutes, such as 90 or 15m, not ${text}`)
  }
  const ms = Math.round(Number(limit[1]) * (limit[2] === 'm' ? 60_000 : 1_000))
  if (ms > LONGEST_TIMER_MS) {
    throw new Error(`--limit takes at most 24 days, not ${text}; it counts seconds, or minutes with an m`)
  }
  return ms
}
