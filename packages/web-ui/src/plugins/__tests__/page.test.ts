import { expect, spyOn, test } from 'bun:test'
import { defineComponent, effectScope, ref, shallowRef, watch } from 'vue'
import { z } from 'zod'
import type { SettingsNavGroup } from '../../settings/types'
import { createOverlayStore } from '../../overlay/overlayStore'
import { dismissToast, toasts } from '../../infra/toast'
import {
  PluginCallError,
  bindPages,
  definePage,
  pageContext,
  settingsPage,
  withPluginSections,
  type PageHost,
  type PanelKind,
} from '../page'

// Cost: plain objects and one effect scope; milliseconds.

const nothing = defineComponent({ render: () => null })

/** A panel session of the test's page. */
interface Notes {
  name: string
  dispose(): void
}

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
    files: unused,
    intents: { open: unused, canOpen: unused },
    panel: { tabs: unused, add: unused },
    openSettings: unused,
    overlays: createOverlayStore(),
    ...parts,
  }
}

test("a plugin's section joins the end of its group, or a group of its own, while the user has the plugin on", () => {
  const section = (id: string, group: string) => ({ group, item: { id, label: id, icon: nothing }, component: nothing })
  const pages = [
    definePage({ plugin: 'skills', settings: section('skills', 'Agent') }),
    definePage({ plugin: 'notes', settings: section('notes', 'Notes') }),
    definePage({ plugin: 'browser' }),
  ]
  const rail: SettingsNavGroup[] = [
    { label: 'Agent', items: [{ id: 'models', label: 'Models', icon: nothing }] },
    { label: 'Workspace', items: [{ id: 'devices', label: 'Devices', icon: nothing }] },
  ]
  const joined = withPluginSections(rail, pages, () => true)
  expect(joined.map((group) => [group.label, group.items.map((item) => item.id)])).toEqual([
    ['Agent', ['models', 'skills']],
    ['Workspace', ['devices']],
    ['Notes', ['notes']],
  ])
  expect(rail[0]!.items.map((item) => item.id)).toEqual(['models'])
  const off = withPluginSections(rail, pages, (plugin) => plugin !== 'skills')
  expect(off[0]!.items.map((item) => item.id)).toEqual(['models'])
  expect(settingsPage(pages, 'skills')).toBe(pages[0]!)
  expect(settingsPage(pages, 'models')).toBeNull()
})

test('a panel session runs for its conversation until the panel lets it go, and the kinds see it', () => {
  const ended: string[] = []
  const shown = ref(0)
  const seen: number[] = []
  const note: PanelKind<string, Notes> = {
    kind: 'note',
    schema: z.string(),
    title: (data, tab) => `${data} in ${tab.session.name}`,
    mark: nothing,
    content: nothing,
  }
  const page = definePage({
    plugin: 'notes',
    kinds: [note],
    panel: (conversation): Notes => {
      // A watch the session starts stops with it.
      watch(shown, (value) => void seen.push(value), { flush: 'sync' })
      return { name: conversation, dispose: () => void ended.push(conversation) }
    },
  })
  const bound = bindPages([page], host(), 'c1')
  expect(bound.kinds[0]!.title('draft')).toBe('draft in c1')
  shown.value = 1
  bound.dispose()
  shown.value = 2
  expect(ended).toEqual(['c1'])
  expect(seen).toEqual([1])
})

test('a page adds tabs only of its own kinds', () => {
  const added: string[] = []
  const page = definePage({
    plugin: 'notes',
    kinds: [{ kind: 'note', schema: z.string(), title: (data: string) => data, mark: nothing, content: nothing }],
  })
  const context = pageContext(host({ panel: { tabs: () => [], add: (_conversation, kind) => void added.push(kind) } }), page)
  context.panel.add('c1', 'note', 'draft')
  expect(() => context.panel.add('c1', 'browser', { url: 'about:blank' })).toThrow('has no kind browser')
  expect(added).toEqual(['note'])
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
