import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { productState } from '../__tests__/product-state'
import type { CredentialDto } from '../api/generated/web-api'

// Settings › Credentials lists each credential with its visible fields and
// nothing of its secret ones, deletes one after asking, and creates one with
// New Credential…. The page is the product's panel on a page of happy-dom;
// the backend is a played channel and a stubbed fetch. A few milliseconds
// each.

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
/** A button's name: its text, or the label of a button drawn as an icon. */
const named = (each: HTMLElement) => each.textContent?.trim() || each.getAttribute('aria-label')
const button = (text: string, within: ParentNode = document) =>
  [...within.querySelectorAll<HTMLElement>('[role="button"]')].find((each) => named(each) === text)
const menuItem = (text: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((each) => each.textContent?.trim() === text)

/** A sign-in kept for an hour, its user name visible and its password secret. */
const linear: CredentialDto = {
  id: 'c1',
  name: 'Linear sign-in',
  note: 'To file issues in the team’s tracker',
  fields: [
    { name: 'user', label: 'User name', kind: 'visible', value: 'alex@example.com' },
    { name: 'password', label: 'Password', kind: 'secret' },
  ],
  keepUntil: new Date(Date.now() + 60 * 60_000).toISOString(),
  lastUsedAt: null,
  createdAt: '2026-10-01T00:00:00.000Z',
  updatedAt: '2026-10-01T00:00:00.000Z',
}

/** The requests the page sent besides the model and vendor catalogs', each answered by `answer` or refused as not in the test. */
function backend(answer: (key: string, body: unknown) => Response | null): string[] {
  const requests: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const key = `${init?.method ?? 'GET'} ${String(input)}`
    if (key === 'GET /api/models') {
      return Response.json({ providers: [], smallModel: null })
    }
    // The vendor catalog the page reads as it starts.
    if (key === 'GET /api/providers/catalog') {
      return Response.json({ subscriptions: [], vendors: [] })
    }
    requests.push(key)
    const body = init?.body === undefined ? undefined : JSON.parse(String(init.body))
    return answer(key, body)
      ?? Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `Not in this test: ${key}` }, { status: 404 })
  }) as typeof fetch
  return requests
}

const buttons = () => [...document.querySelectorAll<HTMLElement>('[role="button"]')].map(named)

// A secret field shows nothing on the page, neither its label nor a mask nor
// a control that reads it, and the page asks the backend for nothing; a
// visible field shows its label and value. Planted defects: a secret field
// listed with its label; a row with Edit or a reveal control.
test('a credential lists its visible fields and nothing of a secret one, and offers only Delete', async () => {
  const requests = backend(() => null)
  const { app, product, page } = await openPanel([linear])
  await waitFor(() => page.textContent?.includes('Linear sign-in') === true)
  const text = page.textContent!
  expect(text).toContain('To file issues in the team’s tracker')
  expect(text).toContain('User name')
  expect(text).toContain('alex@example.com')
  expect(text).not.toContain('Password')
  expect(text).not.toContain('•')
  expect(page.querySelectorAll('input').length).toBe(0)
  expect(buttons()).toEqual(['New Credential…', 'Delete Linear sign-in'])
  expect(requests).toEqual([])
  app.unmount()
  product.stop()
}, 2000)

// Delete… asks first, and Delete in the question removes the credential from
// the backend and from the list. Planted defect: a confirmation that sends
// nothing.
test('Delete… asks first, then deletes the credential and its row goes', async () => {
  const requests = backend((key) => (key === 'DELETE /api/credentials/c1' ? new Response(null, { status: 204 }) : null))
  const { app, product, page } = await openPanel([linear])
  await waitFor(() => button('Delete Linear sign-in') !== undefined)
  button('Delete Linear sign-in')!.click()
  await waitFor(() => dialog()?.textContent?.includes('Delete “Linear sign-in”?') === true)
  expect(requests).toEqual([])
  button('Delete', dialog()!)!.click()
  await waitFor(() => !page.textContent!.includes('Linear sign-in'))
  await waitFor(() => dialog() === null)
  expect(requests).toEqual(['DELETE /api/credentials/c1'])
  expect(page.textContent).toContain('No credentials')
  app.unmount()
  product.stop()
}, 2000)

// New Credential… creates a credential, kept always, with the fields as its
// rows and their ⋯ menus left them. Planted defects: a kind the menu does not
// change; Delete Field that keeps the field.
test('New Credential… creates a credential with the fields its menus left', async () => {
  const bodies: unknown[] = []
  backend((key, body) => {
    if (key !== 'POST /api/credentials') {
      return null
    }
    bodies.push(body)
    return Response.json({ credential: { ...linear, id: 'c2', name: 'Deploy host' } }, { status: 201 })
  })
  const { app, product } = await openPanel([])
  await waitFor(() => button('New Credential…') !== undefined)
  button('New Credential…')!.click()
  await waitFor(() => dialog()?.textContent?.includes('New Credential') === true)
  const editor = dialog()!
  const input = (label: string) => editor.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!
  const type = (label: string, value: string) => {
    input(label).value = value
    input(label).dispatchEvent(new Event('input'))
  }
  type('Name', 'Deploy host')
  type('Label of field 1', 'Token')
  type('Value of field 1', 'tok-1')
  button('Add Field', editor)!.click()
  await waitFor(() => editor.querySelector('input[aria-label="Label of field 2"]') !== null)
  type('Label of field 2', 'Host')
  type('Value of field 2', 'deploy.example.com')

  editor.querySelector<HTMLElement>('[aria-label="Actions for field 2"]')!.click()
  await waitFor(() => menuItem('Visible') !== undefined)
  expect(menuItem('Secret')!.getAttribute('aria-checked')).toBe('true')
  menuItem('Visible')!.click()
  await waitFor(() => menuItem('Visible') === undefined)

  button('Add Field', editor)!.click()
  await waitFor(() => editor.querySelector('[aria-label="Actions for field 3"]') !== null)
  editor.querySelector<HTMLElement>('[aria-label="Actions for field 3"]')!.click()
  await waitFor(() => menuItem('Delete Field') !== undefined)
  menuItem('Delete Field')!.click()
  await waitFor(() => editor.querySelector('input[aria-label="Label of field 3"]') === null)

  button('Create Credential', editor)!.click()
  await waitFor(() => bodies.length === 1)
  expect(bodies[0]).toEqual({
    name: 'Deploy host',
    note: '',
    keep: 'always',
    fields: [
      { name: 'token', label: 'Token', kind: 'secret', value: 'tok-1' },
      { name: 'host', label: 'Host', kind: 'visible', value: 'deploy.example.com' },
    ],
  })
  await waitFor(() => dialog() === null)
  app.unmount()
  product.stop()
}, 2000)
