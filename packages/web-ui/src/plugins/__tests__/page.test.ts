import { createPinia } from 'pinia'
import { expect, spyOn, test } from 'bun:test'
import { defineComponent, effectScope, nextTick, ref, shallowRef } from 'vue'
import { z } from 'zod'
import type { SettingsNavGroup } from '../../settings/types'
import { createOverlayStore } from '../../overlay/overlayStore'
import { dismissToast, toasts } from '../../infra/toast'
import {
  PluginCallError,
  definePage,
  pageContext,
  settingsPage,
  useBoundKinds,
  withPluginSections,
  type KindTab,
  type PageHost,
} from '../page'

// Cost: plain objects and one effect scope; milliseconds.

const nothing = defineComponent({ render: () => null })

/** A host whose every part a test may replace; the rest refuses to be used. */
function host(parts: Partial<PageHost> = {}): PageHost {
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
    panel: { tabs: unused, select: unused, canGo: unused, go: unused },
    openSettings: unused,
    openLink: unused,
    overlays: createOverlayStore(),
    pinia: createPinia(),
    ...parts,
  }
}

test("a plugin's section joins the end of its group while the user has the plugin on", () => {
  const section = (id: string, group: 'agent' | 'workspace') => ({ group, item: { id, label: id, icon: nothing }, component: nothing })
  const pages = [
    definePage({ plugin: 'skills', settings: section('skills', 'agent') }),
    definePage({ plugin: 'notes', settings: section('notes', 'workspace') }),
    definePage({ plugin: 'browser' }),
  ]
  const rail: SettingsNavGroup[] = [
    { id: 'agent', items: [{ id: 'models', label: 'Models', icon: nothing }] },
    { id: 'workspace', items: [{ id: 'devices', label: 'Devices', icon: nothing }] },
  ]
  const joined = withPluginSections(rail, pages, () => true)
  expect(joined.map((group) => [group.id, group.items.map((item) => item.id)])).toEqual([
    ['agent', ['models', 'skills']],
    ['workspace', ['devices', 'notes']],
  ])
  expect(rail[0]!.items.map((item) => item.id)).toEqual(['models'])
  const off = withPluginSections(rail, pages, (plugin) => plugin !== 'skills')
  expect(off[0]!.items.map((item) => item.id)).toEqual(['models'])
  expect(settingsPage(pages, 'skills')).toBe(pages[0]!)
  expect(settingsPage(pages, 'models')).toBeNull()
})

/**
 * Planted defect this catches: the panel binding every page anew on any
 * switch, as it did, which remounts the other plugins' tabs.
 */
test('turning one plugin off and on binds its kinds alone, and another conversation binds every page anew', async () => {
  const page = (plugin: string) =>
    definePage({
      plugin,
      kinds: [{ kind: plugin, schema: z.string(), title: (data: string, tab: KindTab) => `${data} in ${tab.conversation}`, mark: nothing, content: nothing }],
    })
  const pages = [page('notes'), page('browser')]
  const on = ref(new Set(['notes', 'browser']))
  const conversation = ref<string | null>('c1')
  const scope = effectScope()
  const kinds = scope.run(() => useBoundKinds(() => pages.filter((entry) => on.value.has(entry.plugin)), host(), () => conversation.value))!
  const [notes, browser] = kinds.value
  expect(kinds.value.map((kind) => kind.title('draft'))).toEqual(['draft in c1', 'draft in c1'])

  on.value = new Set(['notes'])
  await nextTick()
  expect(kinds.value.map((kind) => kind.kind)).toEqual(['notes'])
  on.value = new Set(['notes', 'browser'])
  await nextTick()
  // The notes' kind is the one bound before, so its tabs never remounted; the browser's is bound anew.
  expect(kinds.value[0]).toBe(notes)
  expect(kinds.value[1]).not.toBe(browser)

  conversation.value = 'c2'
  await nextTick()
  expect(kinds.value[0]).not.toBe(notes)
  expect(kinds.value.map((kind) => kind.title('draft'))).toEqual(['draft in c2', 'draft in c2'])
  conversation.value = null
  await nextTick()
  expect(kinds.value).toEqual([])
  scope.stop()
})

test('a page selects tabs only of its own kinds', () => {
  const selected: string[] = []
  const page = definePage({
    plugin: 'notes',
    kinds: [{ kind: 'note', schema: z.string(), title: (data: string) => data, mark: nothing, content: nothing }],
  })
  const context = pageContext(host({ panel: { tabs: () => [], select: (_conversation, kind) => void selected.push(kind), canGo: () => false, go: () => {} } }), page)
  context.panel.select('c1', 'note', 'n1')
  expect(() => context.panel.select('c1', 'browser', 'b1')).toThrow('has no kind browser')
  expect(selected).toEqual(['note'])
})

test('a page that declares one kind twice is refused', () => {
  const kind = { kind: 'note', schema: z.string(), title: (data: string) => data, mark: nothing, content: nothing }
  expect(() => definePage({ plugin: 'notes', kinds: [kind, kind] })).toThrow('declares the kind note twice')
})

/** The Skills state as one build of the page reads it, which another build of the backend may not send. */
const skillsState = z.object({
  sources: z.array(z.object({ id: z.string(), updateAvailable: z.boolean() })),
})

test('a user state the page cannot read is reported once, and the page goes on with the last one it read', () => {
  for (const toast of [...toasts]) dismissToast(toast.id)
  const logged = spyOn(console, 'error').mockImplementation(() => {})
  const sent = shallowRef<unknown>({ sources: [{ id: 'src_1', updateAvailable: false }] })
  const context = pageContext(host({ userState: () => sent.value }), definePage({ plugin: 'skills' }))
  const scope = effectScope()
  const state = scope.run(() => context.plugin.state(skillsState))!
  expect(state.value?.sources.map((source) => source.id)).toEqual(['src_1'])

  // A backend of an older build sends the state without `updateAvailable`,
  // and again after each change: reading it never throws into the page.
  sent.value = { sources: [{ id: 'src_2' }] }
  expect(state.value?.sources.map((source) => source.id)).toEqual(['src_1'])
  sent.value = { sources: [{ id: 'src_3' }] }
  expect(state.value?.sources.map((source) => source.id)).toEqual(['src_1'])
  // The toast speaks plainly; what did not read is for a developer, in the
  // console.
  expect(toasts.map((toast) => [toast.title, toast.message, toast.tone])).toEqual([
    ['Could Not Read the Plugin’s Data', 'Reloading the page may help.', 'danger'],
  ])
  expect(logged.mock.calls.flat().join(' ')).toContain('updateAvailable')

  sent.value = { sources: [{ id: 'src_2', updateAvailable: true }] }
  expect(state.value?.sources.map((source) => source.id)).toEqual(['src_2'])
  expect(toasts).toHaveLength(1)
  scope.stop()
  logged.mockRestore()
  for (const toast of [...toasts]) dismissToast(toast.id)
})

test("a plugin's user state the page cannot read toasts once however many components read it", () => {
  for (const toast of [...toasts]) dismissToast(toast.id)
  const logged = spyOn(console, 'error').mockImplementation(() => {})
  const sent = shallowRef<unknown>({ sources: [{ id: 'src_1', updateAvailable: false }] })
  // One page of the shell reads the state in two places, and a second page
  // context over the same host, as a second window of the settings would
  // make, reads it as well.
  const shared = host({ userState: () => sent.value })
  const first = pageContext(shared, definePage({ plugin: 'skills' }))
  const second = pageContext(shared, definePage({ plugin: 'skills' }))
  const readers = [effectScope(), effectScope(), effectScope()]
  readers[0]!.run(() => first.plugin.state(skillsState))
  readers[1]!.run(() => first.plugin.state(skillsState))
  readers[2]!.run(() => second.plugin.state(skillsState))

  sent.value = { sources: [{ id: 'src_2' }] }
  expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Read the Plugin’s Data'])

  // A reader that goes while the others still cannot read says nothing more.
  readers[0]!.stop()
  readers[1]!.stop()
  sent.value = { sources: [{ id: 'src_3' }] }
  expect(toasts).toHaveLength(1)

  // Once the state reads again, its next refusal is news.
  sent.value = { sources: [{ id: 'src_3', updateAvailable: false }] }
  sent.value = { sources: [{ id: 'src_4' }] }
  expect(toasts).toHaveLength(2)
  readers[2]!.stop()
  logged.mockRestore()
  for (const toast of [...toasts]) dismissToast(toast.id)
})

test('a call its caller dropped, as a panel that closed, is neither a toast nor a defect', () => {
  for (const toast of [...toasts]) dismissToast(toast.id)
  const logged = spyOn(console, 'error').mockImplementation(() => {})
  const context = pageContext(host(), definePage({ plugin: 'browser' }))
  const left = new AbortController()
  left.abort()
  context.errors.report('Could Not Reload the Page', left.signal.reason)
  context.errors.defect('The panel could not look for a closed tab', left.signal.reason)
  expect(toasts).toEqual([])
  expect(logged).not.toHaveBeenCalled()
  // Any other failure still says so.
  context.errors.report('Could Not Reload the Page', new Error('The device is offline.'))
  expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Reload the Page'])
  logged.mockRestore()
  for (const toast of [...toasts]) dismissToast(toast.id)
})

test("a conversation state the page cannot read is the state's error until one reads, and the last one stays", () => {
  const sent = shallowRef<unknown>({ sources: [{ id: 'src_1', updateAvailable: false }] })
  const context = pageContext(
    host({
      followState: () => ({ value: () => sent.value, error: () => null, read: () => {}, stop: () => {} }),
    }),
    definePage({ plugin: 'notes' }),
  )
  const scope = effectScope()
  const state = scope.run(() => context.plugin.conversation('c1').state(skillsState))!
  sent.value = { sources: [{ id: 'src_2' }] }
  expect(state.value.value?.sources.map((source) => source.id)).toEqual(['src_1'])
  expect(state.error.value).toBeInstanceOf(PluginCallError)
  expect(state.error.value?.reason).toBe('unreadable_state')
  sent.value = { sources: [{ id: 'src_2', updateAvailable: false }] }
  expect(state.value.value?.sources.map((source) => source.id)).toEqual(['src_2'])
  expect(state.error.value).toBeNull()
  scope.stop()
})
