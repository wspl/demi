import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { productState } from '../__tests__/product-state'

// Settings › Credentials asks Confirm It’s You before the eye shows a secret
// when the backend answers that the browser has not confirmed in the last
// five minutes, and reveals once the user has; within those five minutes the
// backend answers the values, and the eye asks nothing (`credentials.md`).
// The page is the product's panel on a page of happy-dom; the backend is a
// played channel and a stubbed fetch. A few milliseconds.

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
})

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
      return Response.json({ providers: [], smallModel: null, imageEntries: {} })
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
  channels.last().connect(productState({
    credentials: [{
      id: 'c1',
      name: 'Linear sign-in',
      note: '',
      fields: [{ name: 'password', label: 'Password', kind: 'secret' }],
      keepUntil: null,
      lastUsedAt: null,
      createdAt: '2026-10-01T00:00:00.000Z',
      updatedAt: '2026-10-01T00:00:00.000Z',
    }],
  }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp(CredentialsPanel)
  app.use(pinia)
  app.mount(page)
  const eye = () => document.querySelector<HTMLElement>('[aria-label="Show Password"]')
  const dialog = () => document.querySelector<HTMLElement>('[role="dialog"]')
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
