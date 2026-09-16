/** A timestamp that many milliseconds ago, so fixtures read as "just now" whenever they run. */
export function ago(ms: number): string {
  return new Date(Date.now() - ms).toISOString()
}

/** A timestamp that many milliseconds from now, so a countdown starts wherever the specimen is opened. */
export function ahead(ms: number): string {
  return new Date(Date.now() + ms).toISOString()
}
