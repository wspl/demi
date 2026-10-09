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
