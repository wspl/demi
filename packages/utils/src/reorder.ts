/** Move an existing item before another item, or to the end. Invalid targets leave order intact. */
export function moveBefore<T>(items: readonly T[], item: T, before: T | null): T[] {
  if (!items.includes(item) || item === before || (before !== null && !items.includes(before)))
    return [...items]
  const next = items.filter((entry) => entry !== item)
  next.splice(before === null ? next.length : next.indexOf(before), 0, item)
  return next
}
