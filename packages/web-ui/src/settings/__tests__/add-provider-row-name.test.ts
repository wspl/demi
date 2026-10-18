import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A screen reader reads each vendor of Add Provider by its name once: the
// mark beside the name, or the initial of a vendor without one, is hidden
// from it (`VendorMark`). One Vue app on a happy-dom page; a few ms besides
// the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { createOverlayStore } = await import('../../overlay/overlayStore')
const { default: AddProviderDialog } = await import('../AddProviderDialog.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** The text assistive technology reads in `node`: hidden parts left out. */
function spoken(node: Node): string {
  if (node instanceof Element && node.getAttribute('aria-hidden') === 'true')
    return ''
  if (node instanceof Element && node.hasAttribute('aria-label'))
    return node.getAttribute('aria-label')!
  if (node.nodeType === Node.TEXT_NODE)
    return node.textContent ?? ''
  return [...node.childNodes].map(spoken).join(' ')
}

test('each vendor row is read by its name once, with or without a mark', async () => {
  const vendor = (id: string, name: string, logo: string | null) => ({
    id, name, wireApi: 'openai-chat' as const, baseUrl: 'https://example.com/v1', logo,
  })
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () =>
      h(AddProviderDialog, {
        isOpen: true,
        overlayStore: createOverlayStore(),
        vendors: [vendor('volcengine', 'Volcengine Ark', '/logos/volcengine.svg'), vendor('chutes', 'Chutes', null)],
        load: 'ready',
      }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()

  const rows = [...document.querySelectorAll('button')]
    .map((button) => spoken(button).replace(/\s+/g, ' ').trim())
    .filter((name) => name.includes('Volcengine') || name.includes('Chutes'))
  expect(rows).toEqual(['Volcengine Ark OpenAI Chat Completions', 'Chutes OpenAI Chat Completions'])
})
