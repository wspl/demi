import { expect, test } from 'bun:test'
import { createPinia } from 'pinia'
import { onScopeDispose, ref, watch } from 'vue'
import { definePage, pageContext, type PageHost } from '../page'
import { definePluginStore, disposePluginStores } from '../stores'

// Cost: one Pinia and plain objects; milliseconds.

/** A host of which the stores use only its Pinia. */
function host(): PageHost {
  const unused = () => {
    throw new Error('the test does not use this part of the host')
  }
  return {
    userState: unused,
    followState: unused,
    call: unused,
    stream: unused,
    installed: unused,
    hostStarting: unused,
    files: unused,
    intents: { open: unused, canOpen: unused },
    panel: { tabs: unused, select: unused },
    openSettings: unused,
    openLink: unused,
    overlays: { open: unused } as never,
    pinia: createPinia(),
  }
}

/**
 * Planted defects this catches: a store made again at each use, which loses
 * what the page keeps, and a switch that disposes every plugin's stores.
 */
test('a store lives from its first use until its plugin is turned off, and another plugin’s store stays', () => {
  const ended: string[] = []
  const tick = ref(0)
  const seen: string[] = []
  const views = (plugin: string) =>
    definePluginStore('views', () => {
      watch(tick, () => void seen.push(plugin), { flush: 'sync' })
      onScopeDispose(() => void ended.push(plugin))
      return { kept: new Map<string, string>() }
    })
  const shared = host()
  const browser = pageContext(shared, definePage({ plugin: 'browser' }))
  const mobile = pageContext(shared, definePage({ plugin: 'mobile' }))
  const browserViews = views('browser')
  const mobileViews = views('mobile')
  browser.store(browserViews).kept.set('t1', 'picture')
  mobile.store(mobileViews).kept.set('d1', 'frame')
  expect(browser.store(browserViews).kept.get('t1')).toBe('picture')

  disposePluginStores(shared.pinia, 'browser')
  tick.value += 1
  expect(ended).toEqual(['browser'])
  expect(seen).toEqual(['mobile'])
  expect(mobile.store(mobileViews).kept.get('d1')).toBe('frame')
  // Turned on again, the plugin starts with a store of nothing.
  expect(browser.store(browserViews).kept.size).toBe(0)
})
