import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
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
const channels = playChannels()

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
