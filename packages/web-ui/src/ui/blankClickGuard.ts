export function createBlankClickGuard(windowMs = 200) {
  let suppressedUntil = 0

  return {
    suppress(now = Date.now()) {
      suppressedUntil = now + windowMs
    },
    isSuppressed(now = Date.now()) {
      return now < suppressedUntil
    },
  }
}
