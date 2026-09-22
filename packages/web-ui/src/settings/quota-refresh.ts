/** Reuses an account's last quota request for one minute across quota displays. */
export function createQuotaRefreshCache() {
  const completed = new Map<string, number>()
  return {
    isFresh(providerId: string, accountId: string): boolean {
      const at = completed.get(JSON.stringify([providerId, accountId]))
      return at !== undefined && Date.now() - at < 60_000
    },
    record(providerId: string, accountId: string): void {
      completed.set(JSON.stringify([providerId, accountId]), Date.now())
    },
    clear(): void {
      completed.clear()
    },
  }
}
