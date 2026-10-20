import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A conversation on a device since removed shows the plain computer without a
// state dot, in its header's host button as in its sidebar row: the page no longer
// knows the device's state, though the conversation still names it. The button is
// Run On's trigger, fed as the product feeds it, on a page of happy-dom. A few
// milliseconds, besides the start of the page.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h } = await import('vue')
const { createPageI18n } = await import('@demicodes/web-ui/i18n/text')
const { hostGlyph } = await import('@demicodes/web-ui/devices/device-glyphs')
const { useProduct } = await import('../state/product')
const { primaryHostOf, executionFor } = await import('./execution')
const { productState } = await import('../__tests__/product-state')
const { createPinia, disposePinia, setActivePinia } = await import('pinia')

const pages: (() => void)[] = []
const pinia = createPinia()
setActivePinia(pinia)
afterEach(() => {
  pages.splice(0).forEach((unmount) => unmount())
  disposePinia(pinia)
})

test('a conversation on a removed device shows the plain computer without a state dot', async () => {
  const i18n = createPageI18n('en')
  const element = document.createElement('div')
  document.body.append(element)
  // The account lists no device by the conversation's id any more.
  useProduct().read = productState({ devices: [] })
  const host = primaryHostOf(executionFor({
    target: { kind: 'device', deviceId: 'gone', path: '/Users/alex' },
    cwd: '/Users/alex',
  }))
  const app = createApp({ render: () => h(hostGlyph(host)) })
  app.use(i18n)
  app.use(pinia)
  app.mount(element)
  pages.push(() => app.unmount())

  expect(element.querySelector('svg')).not.toBeNull()
  expect(element.querySelectorAll('.rounded-full')).toHaveLength(0)
})
