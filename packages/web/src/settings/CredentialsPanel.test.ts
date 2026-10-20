import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { productState } from '../__tests__/product-state'
import type { CredentialDto } from '../api/generated/web-api'

// Settings › Credentials asks Confirm It’s You before the eye shows a secret
// when the backend answers that the browser has not confirmed in the last
// five minutes, and reveals once the user has; within those five minutes the
// backend answers the values, and the eye asks nothing. The page is the
// product's panel on a page of happy-dom; the backend is a played channel
// and a stubbed fetch. A few milliseconds.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/settings/credentials' })
// After the page, since the components ask for its elements and the played
// channel is one of its event targets.
const { playChannels } = await import('../__tests__/sync-channel')
await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp } = await import('vue')
const { createPinia } = await import('pinia')
const { waitFor } = await import('@demicodes/utils')
const { useSession } = await import('../auth/session')
const { useProduct } = await import('../state/product')
const { default: CredentialsPanel } = await import('./CredentialsPanel.vue')

const realFetch = globalThis.fetch
let channels: ReturnType<typeof playChannels>

beforeEach(() => {
  channels = playChannels()
})

afterEach(() => {
  globalThis.fetch = realFetch
  channels.restore()
  document.body.replaceChildren()
})

/** The product's panel on the page, signed in as Alex, with `credentials` as the channel brings them. */
async function openPanel(credentials: CredentialDto[]) {
  const pinia = createPinia()
  const product = useProduct(pinia)
  useSession(pinia).current = {
    status: 'signedIn',
    user: {
      id: 'alex',
      email: 'alex@example.test',
      nickname: 'Alex',
      role: 'master',
      createdAt: '2026-09-01T00:00:00.000Z',
      disabled: false,
      choosesPassword: false,
    },
  }
  product.start()
  channels.last().connect(productState({ credentials }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp(CredentialsPanel)
  app.use(pinia)
  app.mount(page)
  return { app, pinia, product, page }
}

const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')
const button = (text: string, within: ParentNode = document) =>
  [...within.querySelectorAll<HTMLElement>('[role="button"]')].find((each) => each.textContent?.trim() === text)
const menuItem = (text: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((each) => each.textContent?.trim() === text)

test('the eye asks the user to confirm it is them, then shows the secret, and a second eye within the window asks nothing', async () => {
  const requests: string[] = []
  /** The values route's answers in turn: refused until confirmed, then the values. */
  const reveals: Array<() => Response> = [
    () => Response.json({ requestId: 'req_test000000000001', code: 'verification_required', message: 'Confirm it is you first' }, { status: 403 }),
    () => Response.json({ values: { password: 'hunter2-secret' } }),
    () => Response.json({ values: { password: 'hunter2-secret' } }),
  ]
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`
    if (key === 'GET /api/models') {
      return Response.json({ providers: [], smallModel: null })
    }
    requests.push(init?.body === undefined ? key : `${key} ${String(init.body)}`)
    if (key === 'GET /api/credentials/c1/values') {
      return reveals.shift()!()
    }
    if (key === 'POST /api/auth/verify') {
      return new Response(null, { status: 204 })
    }
    return Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `Not in this test: ${key}` }, { status: 404 })
  }) as typeof fetch
  const { app, pinia, product, page } = await openPanel([{
    id: 'c1',
    name: 'Linear sign-in',
    note: '',
    fields: [{ name: 'password', label: 'Password', kind: 'secret' }],
    keepUntil: null,
    lastUsedAt: null,
    createdAt: '2026-10-01T00:00:00.000Z',
    updatedAt: '2026-10-01T00:00:00.000Z',
  }])
  const eye = () => document.querySelector<HTMLElement>('[aria-label="Show Password"]')
  const shown = () => document.body.textContent?.includes('hunter2-secret') === true
  await waitFor(() => eye() !== null)

  // Not confirmed in the last five minutes: the eye asks first and shows nothing.
  eye()!.click()
  await waitFor(() => dialog()?.textContent?.includes('Confirm It’s You') === true)
  expect(dialog()!.textContent).toContain('Confirm it’s you to show a secret.')
  expect(shown()).toBe(false)
  const password = dialog()!.querySelector<HTMLInputElement>('input[type="password"]')!
  password.value = 'master-pass-1'
  password.dispatchEvent(new Event('input'))
  // Continue, the dialog's default action.
  dialog()!.querySelector<HTMLElement>('[data-default-action]')!.click()
  await waitFor(shown)
  await waitFor(() => dialog() === null)
  expect(requests).toEqual([
    'GET /api/credentials/c1/values',
    'POST /api/auth/verify {"password":"master-pass-1"}',
    'GET /api/credentials/c1/values',
  ])

  // Settings opened again within the window drops what the eye showed; the
  // backend answers the values, and the eye asks nothing.
  app.unmount()
  const again = createApp(CredentialsPanel)
  again.use(pinia)
  again.mount(page)
  await waitFor(() => eye() !== null)
  expect(shown()).toBe(false)
  eye()!.click()
  await waitFor(shown)
  expect(dialog()).toBeNull()
  expect(requests.slice(3)).toEqual(['GET /api/credentials/c1/values'])
  again.unmount()
  product.stop()
}, 2000)

/** A sign-in kept for an hour, its user name visible and its password secret. */
const linear: CredentialDto = {
  id: 'c1',
  name: 'Linear sign-in',
  note: '',
  fields: [
    { name: 'user', label: 'User name', kind: 'visible', value: 'alex@example.com' },
    { name: 'password', label: 'Password', kind: 'secret' },
  ],
  keepUntil: new Date(Date.now() + 60 * 60_000).toISOString(),
  lastUsedAt: null,
  createdAt: '2026-10-01T00:00:00.000Z',
  updatedAt: '2026-10-01T00:00:00.000Z',
}

/** The bodies the page PATCHed to the credential; each answers the credential unchanged. */
function patches(): unknown[] {
  const bodies: unknown[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`
    if (key === 'GET /api/models') {
      return Response.json({ providers: [], smallModel: null })
    }
    if (key === 'PATCH /api/credentials/c1') {
      bodies.push(JSON.parse(String(init!.body)))
      return Response.json({ credential: linear })
    }
    return Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `Not in this test: ${key}` }, { status: 404 })
  }) as typeof fetch
  return bodies
}

/** Opens the editor of the panel's one credential. */
async function edit(): Promise<HTMLElement> {
  await waitFor(() => button('Edit') !== undefined)
  button('Edit')!.click()
  await waitFor(() => dialog()?.textContent?.includes('Edit Credential') === true)
  return dialog()!
}

// The editor of a credential kept for a while says when it is removed and
// offers Keep Always, which saves it kept always and says nothing more of
// it. Planted defect: Keep Always that saves no keep. A few milliseconds.
test('Keep Always in the editor saves the credential kept always', async () => {
  const bodies = patches()
  const { app, product } = await openPanel([linear])
  const editor = await edit()
  expect(editor.textContent).toMatch(/Removed .* at /)
  button('Keep Always', editor)!.click()
  await waitFor(() => !editor.textContent!.includes('Removed'))
  button('Save', editor)!.click()
  await waitFor(() => bodies.length === 1)
  expect(bodies[0]).toMatchObject({ keep: 'always' })
  app.unmount()
  product.stop()
}, 2000)

// A field's ⋯ menu makes it secret or visible and deletes it; Save sends the
// fields as the menu left them, and no keep when Keep Always was not chosen.
// Planted defects: a kind the menu does not change; Delete Field that keeps
// the field. A few milliseconds.
test('a field’s menu makes it visible and deletes another, and Save sends the fields so', async () => {
  const bodies = patches()
  const { app, product } = await openPanel([linear])
  const editor = await edit()
  const value = (n: number) => editor.querySelector<HTMLInputElement>(`input[aria-label="Value of field ${n}"]`)!
  /** Whether field `n`'s value is hidden: a secret's input has the eye that reveals it. */
  const hidden = (n: number) => value(n).closest('[data-field]')!.querySelector('[aria-label="Show"]') !== null
  expect(hidden(2)).toBe(true)

  editor.querySelector<HTMLElement>('[aria-label="Actions for field 2"]')!.click()
  await waitFor(() => menuItem('Visible') !== undefined)
  expect(menuItem('Secret')!.getAttribute('aria-checked')).toBe('true')
  menuItem('Visible')!.click()
  await waitFor(() => !hidden(2))

  editor.querySelector<HTMLElement>('[aria-label="Actions for field 1"]')!.click()
  await waitFor(() => menuItem('Delete Field') !== undefined)
  menuItem('Delete Field')!.click()
  await waitFor(() => editor.querySelector('input[aria-label="Value of field 2"]') === null)
  expect(editor.querySelector<HTMLInputElement>('input[aria-label="Label of field 1"]')!.value).toBe('Password')

  button('Save', editor)!.click()
  await waitFor(() => bodies.length === 1)
  expect(bodies[0]).toEqual({
    name: 'Linear sign-in',
    note: '',
    fields: [{ name: 'password', label: 'Password', kind: 'visible' }],
  })
  app.unmount()
  product.stop()
}, 2000)
