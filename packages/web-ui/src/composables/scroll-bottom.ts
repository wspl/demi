/** How close to the end a session scroller counts as "at the bottom" (auto-follow re-engages here). */
export const BOTTOM_THRESHOLD_PX = 100

export function distanceFromBottom(el: HTMLElement): number {
  return el.scrollHeight - el.scrollTop - el.clientHeight
}

export function isNearBottom(el: HTMLElement): boolean {
  return distanceFromBottom(el) <= BOTTOM_THRESHOLD_PX
}
