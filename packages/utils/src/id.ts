/** Generates a random UUID. */
export function createId(): string {
  return globalThis.crypto.randomUUID()
}
