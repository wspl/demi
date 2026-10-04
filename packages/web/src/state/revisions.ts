/**
 * A revision the backend counts in memory, with the run of the backend that
 * counted it (`web-api.md` § Revisions counted in memory): a count compares
 * only with counts of the same run.
 */
export interface RunRevision {
  /** The product state's `run` when the page took the revision; null while the page held no snapshot. */
  run: string | null
  revision: number
}

/**
 * Whether `next`, of the run the page holds now, is newer than `held`:
 * anything is newer than nothing, and any revision of the current run is
 * newer than one of another run, since that backend's counts started again.
 */
export function isNewer(next: RunRevision, held: RunRevision | null): boolean {
  return held === null || next.run !== held.run || next.revision > held.revision
}
