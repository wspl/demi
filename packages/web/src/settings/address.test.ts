import { expect, test } from 'bun:test'
import { defineComponent } from 'vue'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'
import { closeSettings, openSettings, pageUnderSettings, routeDetail, routeSection, SETTINGS_ROUTE, showSection } from './address'

// Cost: a memory router, no DOM; milliseconds.

const Page = defineComponent({ render: () => null })

async function routerAt(path: string): Promise<Router> {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/chat/:id?', component: Page },
      { path: '/settings/:section?/:detail?', name: SETTINGS_ROUTE, component: Page },
    ],
  })
  await router.push(path)
  await router.isReady()
  return router
}

/** The next navigation's end, for one the history makes (Back, go). */
function navigated(router: Router): Promise<void> {
  return new Promise((resolve) => {
    const stop = router.afterEach(() => {
      stop()
      resolve()
    })
  })
}

const where = (router: Router) => router.currentRoute.value.fullPath

test('settings open over the page, each section is a step Back undoes, and closing returns to the page', async () => {
  const router = await routerAt('/chat/c-1')
  await openSettings(router, 'account')
  expect(where(router)).toBe('/settings/account')
  expect(pageUnderSettings(router)).toBe('/chat/c-1')

  await showSection(router, 'models')
  expect(routeSection(router.currentRoute.value)).toBe('models')
  // Back returns to the section before, still over the same page.
  const back = navigated(router)
  router.back()
  await back
  expect(where(router)).toBe('/settings/account')
  expect(pageUnderSettings(router)).toBe('/chat/c-1')

  await showSection(router, 'devices')
  const closed = navigated(router)
  await closeSettings(router)
  await closed
  expect(where(router)).toBe('/chat/c-1')
  expect(routeSection(router.currentRoute.value)).toBeUndefined()
})

test('a link opens its section over the chat, and closing leaves the chat in its place', async () => {
  const router = await routerAt('/settings/keyboard')
  expect(routeSection(router.currentRoute.value)).toBe('keyboard')
  expect(pageUnderSettings(router)).toBe('/chat')
  await closeSettings(router)
  expect(where(router)).toBe('/chat')
})

test("a phone's back button returns to the list a section was opened from", async () => {
  const router = await routerAt('/chat')
  await openSettings(router)
  expect(routeSection(router.currentRoute.value)).toBeNull()
  await showSection(router, 'general')
  const back = navigated(router)
  await showSection(router, null)
  await back
  expect(where(router)).toBe('/settings')
  // From the list, Back closes settings.
  const closed = navigated(router)
  router.back()
  await closed
  expect(where(router)).toBe('/chat')
})

test('a device’s page is a step inside its section: Back and the section return to the list, and closing returns to the page', async () => {
  const router = await routerAt('/chat/c-1')
  await openSettings(router, 'devices')
  await showSection(router, 'devices', 'mac')
  expect(where(router)).toBe('/settings/devices/mac')
  expect(routeDetail(router.currentRoute.value)).toBe('mac')

  // The page's back button shows the section, which is the entry before.
  const back = navigated(router)
  await showSection(router, 'devices')
  await back
  expect(where(router)).toBe('/settings/devices')
  expect(routeDetail(router.currentRoute.value)).toBeNull()

  await showSection(router, 'devices', 'mac')
  const closed = navigated(router)
  await closeSettings(router)
  await closed
  expect(where(router)).toBe('/chat/c-1')
})
