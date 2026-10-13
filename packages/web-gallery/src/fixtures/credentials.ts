import { reactive } from 'vue'
import type { SettingsCredential } from '@demicodes/web-ui/settings/types'
import { ahead } from './time'

/**
 * A user's credentials, one of each kind the section shows: kept always with
 * a hidden field and a note, kept for a while with a visible and a hidden
 * field, used lately, and never used.
 */
export function demoCredentials(): SettingsCredential[] {
  return [
    {
      id: 'cred-github',
      name: 'GitHub token for deploys',
      note: 'To push the release branches from the deploy script',
      fields: [{ name: 'token', label: 'Token', kind: 'secret', value: null }],
      keepUntil: null,
      lastUsedAt: ahead(-3 * 60 * 60_000),
    },
    {
      id: 'cred-google',
      name: 'Google API key',
      note: 'To add Google as a provider',
      fields: [{ name: 'key', label: 'API key', kind: 'secret', value: null }],
      keepUntil: ahead(40 * 60_000),
      lastUsedAt: null,
    },
    {
      id: 'cred-linear',
      name: 'Linear sign-in for alex',
      note: '',
      fields: [
        { name: 'user', label: 'User name', kind: 'visible', value: 'alex@example.com' },
        { name: 'password', label: 'Password', kind: 'secret', value: null },
      ],
      keepUntil: ahead(26 * 24 * 60 * 60_000),
      lastUsedAt: ahead(-2 * 24 * 60 * 60_000),
    },
  ]
}

/** Every value behind the credentials above, as the reveal reads them; the list carries the visible ones too. */
export const DEMO_CREDENTIAL_VALUES: Record<string, Record<string, string>> = {
  'cred-github': { token: 'ghp_EXAMPLEEXAMPLEEXAMPLE' },
  'cred-google': { key: 'AIza-example-not-a-real-key' },
  'cred-linear': { user: 'alex@example.com', password: 'correct horse battery staple' },
}

/** The Credentials section's state as a specimen holds it. */
export function createCredentialState(credentials: SettingsCredential[] = demoCredentials()) {
  return reactive({ credentials })
}

export type CredentialState = ReturnType<typeof createCredentialState>
