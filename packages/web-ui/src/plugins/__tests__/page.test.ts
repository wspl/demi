import { expect, test } from 'bun:test'
import { defineComponent, ref, watch } from 'vue'
import { z } from 'zod'
import type { SettingsNavGroup } from '../../settings/types'
import { createOverlayStore } from '../../overlay/overlayStore'
import {
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
    installs: unused,
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
    definePage({ plugin: 'expose' }),
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
