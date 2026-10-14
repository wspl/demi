import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { productState } from '../__tests__/product-state'

// A second tab that shows Providers while the first tab's sign-in runs, and
// then leaves the page, leaves that sign-in as it was: only Cancel ends a
// sign-in (`providers.md` § Login and publication). The tab is the product's
// Providers page on a page of happy-dom; the backend is a played channel and
// a stubbed fetch. A few milliseconds.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/settings/models' })
// After the page, since the components ask for its elements and the played
// channel is one of its event targets.
const { playChannels } = await import('../__tests__/sync-channel')
await import('../__tests__/vue-components')
const { createApp, nextTick } = await import('vue')
const { createPinia } = await import('pinia')
const { waitFor } = await import('@demicodes/utils')
const { useProduct } = await import('../state/product')
const { default: ProvidersPanel } = await import('./ProvidersPanel.vue')

const realFetch = globalThis.fetch
let channels: ReturnType<typeof playChannels>

beforeEach(() => {
  channels = playChannels()
})

afterEach(() => {
  globalThis.fetch = realFetch
  channels.restore()
})

test('a tab that shows a running sign-in and then leaves the page does not cancel it', async () => {
  const requests: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    requests.push(`${init?.method ?? 'GET'} ${path}`)
    if (path === '/api/providers/catalog') {
      return Response.json({ subscriptions: [{ providerType: 'codex', configured: false }], vendors: [] })
    }
    return Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `Not in this test: ${path}` }, { status: 404 })
  }) as typeof fetch
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  // The first tab's sign-in, which this tab's snapshot lists.
  channels.last().connect(productState({
    logins: [{
      id: 'running',
      providerId: null,
      providerType: 'codex',
      login: {
        status: 'pending',
        stage: 'waiting_for_user',
        verificationUrl: 'https://auth.openai.com/codex/device',
        userCode: 'HXRV-7K2M',
        needsCode: false,
        codesTaken: 0,
        codeError: null,
        expiresAt: '2099-01-01T00:10:00.000Z',
      },
    }],
  }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp(ProvidersPanel)
  app.use(pinia)
  app.mount(page)
  // This tab shows it too.
  await waitFor(() => document.body.textContent?.includes('HXRV-7K2M') === true)

  // The tab leaves Providers.
  app.unmount()
  await nextTick()
  expect(requests.filter((request) => request.startsWith('DELETE'))).toEqual([])
  product.stop()
}, 2000)

// A tab's unsent code is its own: another tab's paste, and the refusal of
// it, leave what this tab typed; only this tab's code, once the sign-in took
// it, empties the field (`web-api.md` § Subscription accounts). A few
// milliseconds.
test('another tab’s paste and its refusal leave the code this tab typed; this tab’s taken code empties it', async () => {
  const pasted: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    if (path === '/api/providers/catalog') {
      return Response.json({ subscriptions: [{ providerType: 'codex', configured: false }], vendors: [] })
    }
    if (path === '/api/providers/subscription-login/claude/code' && init?.method === 'POST') {
      pasted.push(JSON.parse(String(init.body)).code)
      return Response.json({ code: 2 })
    }
    return Response.json({ requestId: 'req_test000000000001', code: 'not_found', message: `Not in this test: ${path}` }, { status: 404 })
  }) as typeof fetch
  const listed = (codesTaken: number, codeError: string | null) => [{
    id: 'claude',
    providerId: null,
    // The page takes a code the same way whatever the family.
    providerType: 'codex',
    login: {
      status: 'pending' as const,
      stage: 'waiting_for_user' as const,
      verificationUrl: 'https://claude.com/cai/oauth/authorize',
      userCode: null,
      needsCode: true,
      codesTaken,
      codeError,
      expiresAt: '2099-01-01T00:15:00.000Z',
    },
  }]
  const pinia = createPinia()
  const product = useProduct(pinia)
  product.start()
  channels.last().connect(productState({ logins: listed(0, null) }))
  const page = document.createElement('div')
  document.body.append(page)
  const app = createApp(ProvidersPanel)
  app.use(pinia)
  app.mount(page)
  await waitFor(() => document.querySelector('input[aria-label="Code"]') !== null)
  const field = document.querySelector<HTMLInputElement>('input[aria-label="Code"]')!

  // This tab types a code.
  field.value = 'mine#state'
  field.dispatchEvent(new Event('input'))
  await nextTick()

  // Another tab's code is taken, then refused: what this tab typed stays.
  channels.last().send({ type: 'logins', logins: listed(1, null) })
  await waitFor(() => field.disabled)
  expect(field.value).toBe('mine#state')
  channels.last().send({ type: 'logins', logins: listed(1, 'Invalid code') })
  await waitFor(() => !field.disabled)
  expect(field.value).toBe('mine#state')

  // This tab sends it with Return; once the sign-in took it, the field is empty.
  field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
  await waitFor(() => field.value === '')
  expect(pasted).toEqual(['mine#state'])

  app.unmount()
  page.remove()
  product.stop()
}, 2000)
