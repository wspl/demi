import { expect, test } from 'bun:test'
import { defineComponent } from 'vue'
import type { SettingsNavGroup } from '../../settings/types'
import { pluginPageShown, pluginSettingsPage, withPluginSections, type PluginPage } from '../slots'

const Page = defineComponent({ render: () => null })
const icon = defineComponent({ render: () => null })

const pages: PluginPage[] = [
  { plugin: 'skills', settings: { group: 'Agent', item: { id: 'skills', label: 'Skills', icon }, component: Page } },
  { plugin: 'notes', settings: { group: 'Notes', item: { id: 'notes', label: 'Notes', icon }, component: Page } },
  { plugin: 'expose' },
]
const rail: SettingsNavGroup[] = [
  { label: 'Agent', items: [{ id: 'models', label: 'Models', icon }] },
  { label: 'Workspace', items: [{ id: 'devices', label: 'Devices', icon }] },
]

test("a plugin's section joins the end of its group, or a group of its own, while the user has the plugin on", () => {
  const joined = withPluginSections(rail, pages, () => true)
  expect(joined.map((group) => [group.label, group.items.map((item) => item.id)])).toEqual([
    ['Agent', ['models', 'skills']],
    ['Workspace', ['devices']],
    ['Notes', ['notes']],
  ])
  expect(rail[0]!.items.map((item) => item.id)).toEqual(['models'])
  const off = withPluginSections(rail, pages, (page) => page.plugin !== 'skills')
  expect(off[0]!.items.map((item) => item.id)).toEqual(['models'])
  expect(pluginSettingsPage(pages, 'skills')).toBe(Page)
  expect(pluginSettingsPage(pages, 'models')).toBeNull()
})

test('a page fills its slots only while its plugin is on and serves every method and stream it uses', () => {
  const page: PluginPage = { plugin: 'browser', uses: { methods: ['tabs', 'open'], streams: ['browser'] } }
  const served = { id: 'browser', enabled: true, methods: ['tabs', 'open', 'close'], streams: ['browser'] }
  expect(pluginPageShown(page, [served])).toBe(true)
  expect(pluginPageShown(page, [{ ...served, enabled: false }])).toBe(false)
  // A backend whose catalog lacks the browser's package lists neither.
  expect(pluginPageShown(page, [{ ...served, methods: [], streams: [] }])).toBe(false)
  expect(pluginPageShown(page, [])).toBe(false)
  expect(pluginPageShown({ plugin: 'browser' }, [served])).toBe(true)
})
