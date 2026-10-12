import { expect, test } from 'bun:test'
import { projectsNamingTheirHost } from '../project-hosts'
import type { SidebarProject } from '../types'

function onDevice(id: string, name: string, device: string | null): SidebarProject {
  return device === null
    ? { id, name, path: `/${id}`, hostKind: 'device', device: { kind: 'removed' } }
    : { id, name, path: `/${id}`, hostKind: 'device', device: { kind: 'paired', name: device, state: 'online' } }
}

test('a project names its device only beside a project of its name on another host', () => {
  const projects: SidebarProject[] = [
    onDevice('chromium', 'chromium', 'MacBook Pro'),
    onDevice('demi-mac', 'demi', 'MacBook Pro'),
    onDevice('demi-mini', 'demi', 'Mac mini'),
    // A third of the name, on the Cloud, is another host too.
    { id: 'demi-cloud', name: 'demi', path: '/demi', hostKind: 'cloud' },
    onDevice('notes-a', 'notes', 'MacBook Pro'),
    onDevice('notes-b', 'notes', 'MacBook Pro'),
    onDevice('old-a', 'old', null),
    onDevice('old-b', 'old', 'MacBook Pro'),
  ]
  expect([...projectsNamingTheirHost(projects)].sort()).toEqual(['demi-cloud', 'demi-mac', 'demi-mini', 'old-a', 'old-b'])
})
