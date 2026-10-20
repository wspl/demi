import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A port another row of the device exposes is refused by the server; the
// field keeps the typed value to correct, with the refusal under it, as a
// value the field itself refuses (`crates/backend-user-shard/src/shard/exposed_ports.rs`). One
// Vue app on a happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { createOverlayStore } = await import('../../overlay/overlayStore')
const { default: ExposedPortDialog } = await import('../ExposedPortDialog.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

test('a refused port keeps the typed value in its field with why', async () => {
  const refusal = new Error('Port 18910 is already exposed.')
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () =>
      h(ExposedPortDialog, {
        isOpen: true,
        overlayStore: createOverlayStore(),
        exposed: { id: 'exp-1', deviceId: 'mac', port: 11434, label: '', address: 'http://mac:11434' },
        onChange: () => Promise.reject(refusal),
      }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()

  const port = document.querySelector<HTMLInputElement>('input[aria-label="Port"]')!
  port.dispatchEvent(new FocusEvent('focus'))
  port.value = '18910'
  port.dispatchEvent(new Event('input'))
  port.dispatchEvent(new FocusEvent('blur'))
  for (let tick = 0; tick < 5; tick++) {
    await Promise.resolve()
    await nextTick()
  }
  expect(port.value).toBe('18910')
  expect(document.body.textContent).toContain('Port 18910 is already exposed.')
})
