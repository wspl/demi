import { inject, provide, type InjectionKey } from 'vue'

/** What a provider entry holds: a subscription's sign-in, or an API key. */
export type CredentialKind = 'subscription' | 'api_key'

/** The kind of credential the entry `providerId` holds; null for an entry the host does not know, as one removed since. */
export type CredentialKinds = (providerId: string) => CredentialKind | null

const credentialKindsKey: InjectionKey<() => CredentialKinds | undefined> = Symbol('credential-kinds')

/**
 * Offers the records below what each entry holds, read at each render, so
 * a failure's headline names the entry's credential; none where the host
 * does not know its entries, and the records then read the code's own
 * words.
 */
export function provideCredentialKinds(source: () => CredentialKinds | undefined): void {
  provide(credentialKindsKey, source)
}

export function useCredentialKinds(): () => CredentialKinds | undefined {
  return inject(credentialKindsKey, () => undefined)
}
