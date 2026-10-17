import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Safari on an iPhone pushes only to Demi added to the Home Screen
// (`turns.md` § Notifications): there turning notifications on in Settings
// shows how to add Demi instead of the browser's question, and the row then
// says so with How to Add, which shows the steps again. Settings ›
// Notifications on a page of happy-dom at a phone's width; a few ms besides
// the start of the page.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 390, height: 844 })

await import('../../__tests__/vue-components')
const { createApp, h, nextTick, ref } = await import('vue')
const { appOverlayStore } = await import('../../overlay/appOverlay')
const { default: SettingsNotifications } = await import('../SettingsNotifications.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

async function settle(): Promise<void> {
  for (let turn = 0; turn < 3; turn++) {
    await nextTick()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

/** The page, its browser one that `homeScreenIcon` says cannot push outside the Home Screen; how often the browser was asked. */
async function open(homeScreenIcon: string | undefined) {
  let asked = 0
  const enabled = ref(false)
  const app = createApp(() =>
    h(SettingsNotifications, {
      'enabled': enabled.value,
      'onUpdate:enabled': (value: boolean) => { enabled.value = value },
      'turnFinishes': true,
      'turnFails': true,
      'needsPermission': true,
      'agentNotifies': true,
      'permission': homeScreenIcon ? 'unsupported' : 'default',
      'requestPermission': async () => {
        asked += 1
        return 'granted' as const
      },
      homeScreenIcon,
      'overlayStore': appOverlayStore,
    }),
  )
  const host = document.createElement('div')
  document.body.append(host)
  app.mount(host)
  unmount = () => app.unmount()
  await settle()
  return { asked: () => asked, enabled }
}

/** The button of the page that says `label`. */
function button(label: string): HTMLElement {
  const found = [...document.querySelectorAll<HTMLElement>('[role="button"]')].find((element) => element.textContent?.trim() === label)
  expect(found).toBeDefined()
  return found!
}

/** The switch of Browser notifications, the first of the page. */
function browserSwitch(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[role="switch"]')
}

test('in Safari on an iPhone, turning notifications on shows how to add Demi to the Home Screen instead of asking', async () => {
  const page = await open('/icons/apple-touch-icon.png')
  expect(browserSwitch()!.getAttribute('aria-disabled')).not.toBe('true')
  browserSwitch()!.click()
  await settle()

  expect(page.asked()).toBe(0)
  expect(page.enabled.value).toBe(false)
  expect(document.body.textContent).toContain('To get notifications on iPhone, add Demi to your Home Screen')
  expect(document.body.textContent).toContain('Choose Add to Home Screen.')
  expect(document.querySelector('img[src="/icons/apple-touch-icon.png"]')).not.toBeNull()

  // OK closes the steps; the row then says what to do, and How to Add shows them again.
  button('OK').click()
  await settle()
  expect(document.body.textContent).not.toContain('Choose Add to Home Screen.')
  expect(document.body.textContent).toContain('Add Demi to your Home Screen to get notifications')
  button('How to Add').click()
  await settle()
  expect(document.body.textContent).toContain('Choose Add to Home Screen.')
})

test('elsewhere, turning notifications on asks the browser', async () => {
  const page = await open(undefined)
  browserSwitch()!.click()
  await settle()
  expect(page.asked()).toBe(1)
  expect(page.enabled.value).toBe(true)
  expect(document.body.textContent).not.toContain('Home Screen')
})
