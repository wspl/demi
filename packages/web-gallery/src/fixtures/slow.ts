/**
 * A request that takes a moment, for a specimen whose control shows its work
 * while it waits: the promise a specimen's action returns, which the control
 * works until it settles. Nothing holds on to the timer once it has fired.
 */
export function slow(ms = 900): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms)
  })
}
