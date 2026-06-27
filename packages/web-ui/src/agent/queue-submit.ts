export function queuedMessageIdForEmptySubmit(queue: readonly { id: string }[]): string | null {
  if (queue.length === 0) return null
  return queue[queue.length - 1]?.id ?? null
}
