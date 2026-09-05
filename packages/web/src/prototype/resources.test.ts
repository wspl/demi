import { expect, test } from 'bun:test'
import { createPinia, setActivePinia } from 'pinia'
import { useResources } from './resources'

test('recent workspace selection deduplicates without reordering the project sidebar', () => {
  setActivePinia(createPinia())
  const resources = useResources()
  const projectOrder = resources.projects.map((project) => project.id)
  resources.rememberProject('notes')
  resources.rememberProject('demi')
  resources.rememberProject('notes')
  expect(resources.recentProjectIds).toEqual(['notes', 'demi'])
  expect(resources.projects.map((project) => project.id)).toEqual(projectOrder)
})

test('project ordering is separate from recent directory history', () => {
  setActivePinia(createPinia())
  const resources = useResources()
  resources.reorderProject('notes', 'demi')
  expect(resources.projects.map((project) => project.id)).toEqual(['notes', 'demi'])
  resources.rememberProject('demi')
  expect(resources.projects.map((project) => project.id)).toEqual(['notes', 'demi'])
})
