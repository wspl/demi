/** One mutation per provider; a device login holds the same reservation until it settles. */
export class ProviderOperations {
  private readonly active = new Set<string>()

  reserve(providerId: string): (() => void) | null {
    if (this.active.has(providerId)) return null
    this.active.add(providerId)
    return () => { this.active.delete(providerId) }
  }
}
