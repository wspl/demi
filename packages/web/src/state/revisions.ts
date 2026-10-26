/**
 * A revision the backend counts in memory, with the run of the backend that
 * counted it: a count compares only with counts of the same run.
 */
export interface RunRevision {
  /** The backend's run that counted the revision, as the product state names it; null when the page knew none. */
  run: string | null
  revision: number
}

/**
 * Whether `next` is newer than `held`: anything is newer than nothing, and a
 * revision of another run than `held`'s is taken, since a backend's counts
 * start again with each run and the value that came later is the latest.
 */
export function isNewer(next: RunRevision, held: RunRevision | null): boolean {
  return held === null || next.run !== held.run || next.revision > held.revision
}
