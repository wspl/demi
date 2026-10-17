import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A forward's two Host dropdowns show the chosen Host on their closed face
// as Run On's rows draw it (the gallery's Dropdown rule): a device's system's
// logo with its state's dot, the Cloud its cloud; Connect To not yet chosen
// shows the warning in the icon's place. Settings › Port Forwarding's page
// on a page of happy-dom; a few ms besides the start of the page.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })

await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { appOverlayStore } = await import('../../overlay/appOverlay')
const { DEVICE_LOGO_PATH } = await import('../../devices/mark')
const { default: SettingsPortForwardPage } = await import('../SettingsPortForwardPage.vue')
type ForwardHost = import('../port-forwards').ForwardHost
type SettingsPortForward = import('../port-forwards').SettingsPortForward

const HOSTS: ForwardHost[] = [
  { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null },
  { id: 'mac', name: 'alex-mbp', kind: 'device', state: 'online', mark: { logo: 'apple', number: null } },
  { id: 'build', name: 'build-01', kind: 'device', state: 'offline', mark: { logo: 'ubuntu', number: null } },
]

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

async function open(listenDevice: string, connectDevice: string | null): Promise<void> {
  const forward: SettingsPortForward = {
    id: 'f1', name: 'ollama', listenDevice, listenPort: 11434, connectDevice,
    connectAddress: 'localhost:11434', enabled: true, state: null,
  }
  const app = createApp(() =>
    h(SettingsPortForwardPage, {
      forward,
      hosts: HOSTS,
      nameMaxLength: 64,
      overlayStore: appOverlayStore,
      onChange: async () => {},
    }),
  )
  const host = document.createElement('div')
  document.body.append(host)
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
}

/** The closed face of the dropdown named `label`. */
function face(label: string): HTMLElement {
  const found = document.querySelector<HTMLElement>(`[role="button"][aria-label="${label}"]`)
  expect(found).not.toBeNull()
  return found!
}

/** The logo the face draws, by its path; null when it draws none. */
function logoOf(element: HTMLElement): string | null {
  return element.querySelector('svg path')?.getAttribute('d') ?? null
}

test('a closed Host dropdown shows the chosen device\'s logo with its state\'s dot', async () => {
  await open('mac', 'build')

  const listen = face('Listen On')
  expect(logoOf(listen)).toBe(DEVICE_LOGO_PATH.apple)
  expect(listen.querySelector('[role="img"][aria-label="Online"]')).not.toBeNull()
  expect(listen.textContent).toContain('alex-mbp')

  const connect = face('Connect To')
  expect(logoOf(connect)).toBe(DEVICE_LOGO_PATH.ubuntu)
  expect(connect.querySelector('[role="img"][aria-label="Offline"]')).not.toBeNull()
})

test('the Cloud shows its cloud without a dot, and a missing Connect To its warning', async () => {
  await open('cloud', null)

  const listen = face('Listen On')
  expect(listen.querySelector('svg.lucide-cloud')).not.toBeNull()
  expect(listen.querySelector('[data-tone]')).toBeNull()

  const connect = face('Connect To')
  expect(connect.querySelector('svg.lucide-triangle-alert')).not.toBeNull()
  expect(connect.textContent).toContain('Choose a Host')
})
