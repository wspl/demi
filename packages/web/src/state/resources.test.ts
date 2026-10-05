import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { productState } from '../__tests__/product-state'
import { useProduct } from './product'
import { useResources } from './resources'

// Cost: two stores over the generated page registry; milliseconds.

let pinia: ReturnType<typeof createPinia>

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  disposePinia(pinia)
})

test("a plugin page's sidebar entry shows while its plugin is on and opens the settings dialog on its section", () => {
  const product = useProduct()
  const resources = useResources()
  const skills = { id: 'skills', name: 'Skills', description: 'Skills.', enabled: false, packages: [] }

  product.snapshot = productState({ plugins: [skills] })
  expect(resources.sectionEntries).toEqual([])

  product.snapshot = productState({ plugins: [{ ...skills, enabled: true }] })
  expect(resources.sectionEntries.map((entry) => entry.label)).toEqual(['Skills'])
  resources.openSettings(resources.sectionEntries[0]!.section)
  expect(resources.settingsOpen).toBe(true)
  expect(resources.settingsTab).toBe('skills')
})

test("a device project's row follows its device online and offline; a Cloud project has no online state", () => {
  const product = useProduct()
  const resources = useResources()
  const device = {
    id: 'laptop',
    name: 'ZandeMacBook-Pro.local',
    kind: 'user' as const,
    platform: 'darwin' as const,
    claimedAt: '2026-09-10T00:00:00.000Z',
    lastSeenAt: null,
    online: true,
    home: null,
    installs: [],
    installed: [],
  }
  const cloud = { ...device, id: 'cloud', name: 'Cloud', kind: 'managed' as const }
  const workspace = (id: string, deviceId: string) => ({
    id,
    deviceId,
    path: `/${id}`,
    name: id,
    createdAt: '2026-09-10T00:00:00.000Z',
  })
  const workspaces = [workspace('ledger', 'laptop'), workspace('notes', 'cloud')]

  product.snapshot = productState({ devices: [device, cloud], workspaces })
  expect(resources.projects).toEqual([
    { id: 'ledger', name: 'ledger', path: '/ledger', deviceId: 'laptop', host: 'ZandeMacBook-Pro.local', hostKind: 'device', online: true },
    { id: 'notes', name: 'notes', path: '/notes', deviceId: 'cloud', host: 'Cloud', hostKind: 'cloud' },
  ])

  product.snapshot = productState({ devices: [{ ...device, online: false }, cloud], workspaces })
  expect(resources.projects[0]).toMatchObject({ hostKind: 'device', online: false })
})
