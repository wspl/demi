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
      name: 'GitHub token for zan',
      note: 'To push the release branches from the deploy script',
      fields: [{ name: 'token', label: 'Token', kind: 'secret' }],
      keepUntil: null,
      lastUsedAt: ahead(-3 * 60 * 60_000),
      updatedAt: '2026-10-01T09:00:00.000Z',
    },
    {
      id: 'cred-google',
      name: 'Google API key',
      note: 'To add Google as a provider',
      fields: [{ name: 'key', label: 'API key', kind: 'secret' }],
      keepUntil: ahead(40 * 60_000),
      lastUsedAt: null,
      updatedAt: '2026-10-01T09:00:00.000Z',
    },
    {
      id: 'cred-linear',
      name: 'Linear sign-in for zan',
      note: '',
      fields: [
        { name: 'user', label: 'User name', kind: 'visible' },
        { name: 'password', label: 'Password', kind: 'secret' },
      ],
      keepUntil: ahead(26 * 24 * 60 * 60_000),
      lastUsedAt: ahead(-2 * 24 * 60 * 60_000),
      updatedAt: '2026-10-01T09:00:00.000Z',
    },
  ]
}

/** The values behind the credentials above, which only the reveal reads. */
export const DEMO_CREDENTIAL_VALUES: Record<string, Record<string, string>> = {
  'cred-github': { token: 'ghp_EXAMPLEEXAMPLEEXAMPLE' },
  'cred-google': { key: 'AIza-example-not-a-real-key' },
  'cred-linear': { user: 'zan@example.com', password: 'correct horse battery staple' },
}

/** The Credentials section's state as a specimen holds it. */
export function createCredentialState(credentials: SettingsCredential[] = demoCredentials()) {
  return reactive({ credentials })
}

export type CredentialState = ReturnType<typeof createCredentialState>
