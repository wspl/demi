import { reactive } from 'vue'
import type { CredentialLookup, MessageCredentials } from '@demicodes/web-ui/markdown/types'
import type { SettingsCredential } from '@demicodes/web-ui/settings/types'
import { galleryVerification } from './handoff'
import { ahead } from './time'

const DAY = 24 * 60 * 60_000

/**
 * A user's credentials, one of each kind the section shows: kept always with
 * a secret field and a note too long for its line, kept for a while with a
 * visible and a secret field, used lately, never used, and one whose
 * password was never typed.
 */
export function demoCredentials(): SettingsCredential[] {
  return [
    {
      id: 'cred-github',
      name: 'GitHub token for deploys',
      note: 'To push the release branches from the deploy script and tag each release on the example-app repository once its build passes',
      fields: [{ name: 'token', label: 'Token', kind: 'secret', value: null, missing: false }],
      keepUntil: null,
      lastUsedAt: ahead(-3 * 60 * 60_000),
      createdAt: ahead(-90 * DAY),
    },
    {
      id: 'cred-google',
      name: 'Google API key',
      note: 'To add Google as a provider',
      fields: [{ name: 'key', label: 'API key', kind: 'secret', value: null, missing: false }],
      keepUntil: ahead(40 * 60_000),
      lastUsedAt: null,
      createdAt: ahead(-20 * 60_000),
    },
    {
      id: 'cred-linear',
      name: 'Linear sign-in for alex',
      note: '',
      fields: [
        { name: 'user', label: 'User name', kind: 'visible', value: 'alex@example.com', missing: false },
        { name: 'password', label: 'Password', kind: 'secret', value: null, missing: false },
      ],
      keepUntil: ahead(60 * 60_000),
      lastUsedAt: ahead(-2 * DAY),
      createdAt: ahead(-12 * DAY),
    },
    {
      id: 'cred-staging',
      name: 'Staging database',
      note: 'To run the migration checks against staging',
      fields: [
        { name: 'host', label: 'Host', kind: 'visible', value: 'db.staging.example.com', missing: false },
        { name: 'password', label: 'Password', kind: 'secret', value: null, missing: true },
      ],
      keepUntil: null,
      lastUsedAt: null,
      createdAt: ahead(-2 * 60 * 60_000),
    },
  ]
}

/**
 * About two hundred invented credentials of the kinds agents ask for, for the
 * rail's search, sorts and groups at the scale they are for.
 */
export function manyCredentials(): SettingsCredential[] {
  const services = [
    'GitHub', 'GitLab', 'Linear', 'Jira', 'Notion', 'Slack', 'Stripe', 'Vercel', 'Netlify', 'Cloudflare',
    'AWS', 'Google Cloud', 'Azure', 'DigitalOcean', 'Hetzner', 'Sentry', 'Datadog', 'PostHog', 'Figma', 'Airtable',
    'OpenAI', 'Anthropic', 'Replicate', 'Hugging Face', 'Docker Hub', 'npm', 'PyPI', 'Twilio', 'SendGrid', 'Mailgun',
    'Shopify', 'Supabase', 'Firebase', 'MongoDB Atlas', 'Redis Cloud', 'Algolia', 'Contentful', 'Zendesk', 'HubSpot', 'Asana',
  ]
  const kinds = [
    { suffix: 'token for deploys', note: 'To push release branches from the deploy script', fields: [['token', 'Token', 'secret']] },
    { suffix: 'API key', note: 'To call the API from the data import script', fields: [['key', 'API key', 'secret']] },
    { suffix: 'sign-in', note: 'To sign in to the dashboard in the browser', fields: [['user', 'User name', 'visible'], ['password', 'Password', 'secret']] },
    { suffix: 'staging database', note: 'To run the migration checks against staging', fields: [['host', 'Host', 'visible'], ['user', 'User', 'visible'], ['password', 'Password', 'secret']] },
    { suffix: 'webhook secret', note: '', fields: [['secret', 'Secret', 'secret']] },
  ] as const
  const users = ['alex@example.com', 'sam@example.com', 'deploy@example.com', 'ops', 'admin']
  const list: SettingsCredential[] = []
  let n = 0
  for (const service of services) {
    for (const kind of kinds) {
      n += 1
      if ((n * 7) % 9 === 0) {
        continue
      }
      const slug = service.toLowerCase().replace(/\s/g, '-')
      list.push({
        id: `cred-${n}`,
        name: `${service} ${kind.suffix}`,
        note: kind.note,
        fields: kind.fields.map(([name, label, fieldKind], i) => ({
          name,
          label,
          kind: fieldKind,
          value: fieldKind === 'visible' ? (name === 'host' ? `db-${slug}.example.com` : users[(n + i) % users.length]!) : null,
          missing: false,
        })),
        keepUntil: n % 11 === 0 ? ahead(((n % 5) + 1) * DAY) : null,
        lastUsedAt: n % 6 === 0 ? null : ahead(-(((n * 37) % 120) * DAY) / 3),
        createdAt: ahead(-(60 + ((n * 13) % 300)) * DAY),
      })
    }
  }
  return list.sort((a, b) => a.name.localeCompare(b.name))
}

/** The Credentials section's state as a specimen holds it. */
export function createCredentialState(credentials: SettingsCredential[] = demoCredentials()) {
  return reactive({ credentials })
}

export type CredentialState = ReturnType<typeof createCredentialState>

/** The secret values of `demoCredentials`, by credential and field, as the backend would answer them. */
const secrets: Record<string, Record<string, string>> = {
  'cred-github': { token: 'ghp_ExampleOnly0000000000000000000000' },
  'cred-google': { key: 'AIzaExampleOnly-0000000000000000000' },
  'cred-linear': { password: 'correct-horse-battery-staple' },
}

/**
 * `demoCredentials` as a gallery reply's cards read them, as the product's
 * backend would answer: the list is in at once, and a secret's value needs a
 * confirmation that holds five minutes (`galleryVerification`: the password
 * `wrong` is refused, any other confirms).
 */
export function galleryCredentials(): MessageCredentials {
  const saved = demoCredentials()
  const verification = galleryVerification()
  return {
    hold: () => () => {},
    find(name): CredentialLookup {
      const found = saved.find((credential) => credential.name.toLowerCase() === name.toLowerCase())
      return found ? { state: 'found', credential: found } : { state: 'missing' }
    },
    async reveal(id, field) {
      await new Promise((resolve) => setTimeout(resolve, 200))
      if (!verification.verified())
        return { kind: 'needsVerification' }
      const value = secrets[id]?.[field]
      return value === undefined ? { kind: 'failed' } : { kind: 'revealed', value }
    },
    verification: verification.host,
    email: 'alex@example.com',
  }
}

/** A reply that hands the user saved credentials: one field, several, one field of several, and one removed since. */
export const credentialMarkdown = `The token is saved:

<credential name="GitHub token for deploys" />

The Linear sign-in, and its password alone:

<credential name="Linear sign-in for alex" />

<credential name="Linear sign-in for alex" field="password" />

The old token was removed:

<credential name="Old API token" />`
