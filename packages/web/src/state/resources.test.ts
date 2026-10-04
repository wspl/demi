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
