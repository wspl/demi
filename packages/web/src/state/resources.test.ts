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

test("a plugin page's sidebar entry shows while its plugin is on and names its settings section", () => {
  const product = useProduct()
  const resources = useResources()
  const skills = { id: 'skills', name: 'Skills', description: 'Skills.', enabled: false, packages: [] }

  product.snapshot = productState({ plugins: [skills] })
  expect(resources.sectionEntries).toEqual([])

  product.snapshot = productState({ plugins: [{ ...skills, enabled: true }] })
  expect(resources.sectionEntries.map((entry) => [entry.label, entry.section])).toEqual([['Skills', 'skills']])
})

test("a device project's row follows its device online, updating, offline and gone; a Cloud project has no state", () => {
  const product = useProduct()
  const resources = useResources()
  const device = {
    id: 'laptop',
    name: 'ZandeMacBook-Pro.local',
    kind: 'user' as const,
    platform: 'darwin' as const,
    claimedAt: '2026-09-10T00:00:00.000Z',
    lastSeenAt: null,
    state: 'online' as const,
    home: null,
    installed: [],
    startCommand: null,
    os: null,
    runnerVersion: null,
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
    { id: 'ledger', name: 'ledger', path: '/ledger', deviceId: 'laptop', host: 'ZandeMacBook-Pro.local', hostKind: 'device', state: 'online' },
    { id: 'notes', name: 'notes', path: '/notes', deviceId: 'cloud', host: 'Cloud', hostKind: 'cloud' },
  ])

  product.snapshot = productState({ devices: [{ ...device, state: 'updating' }, cloud], workspaces })
  expect(resources.projects[0]).toMatchObject({ hostKind: 'device', state: 'updating' })

  product.snapshot = productState({ devices: [{ ...device, state: 'offline' }, cloud], workspaces })
  expect(resources.projects[0]).toMatchObject({ hostKind: 'device', state: 'offline' })

  // A device the list no longer holds, as for a moment around its
  // revocation, shows as unavailable and offline.
  product.snapshot = productState({ devices: [cloud], workspaces })
  expect(resources.projects[0]).toMatchObject({ host: 'Unavailable device', hostKind: 'device', state: 'offline' })
})
