import type { SidebarDevice, SidebarProject } from './types'

/** What a host is called where the sidebar names it: the Cloud, a device by its name, or a removed one. */
export function projectHostName(project: SidebarProject): string {
  if (project.hostKind === 'cloud') {
    return 'Cloud'
  }
  return deviceName(project.device)
}

function deviceName(device: SidebarDevice): string {
  return device.kind === 'paired' ? device.name : 'Removed device'
}

/**
 * The projects whose row names its device in words: those that share their name with a project on
 * another host, where the mark alone cannot tell them apart. Two projects of one name on one
 * host are not told apart by it, so they do not count (`product.md` § Conversations and projects).
 */
export function projectsNamingTheirHost(projects: readonly SidebarProject[]): Set<string> {
  const hostsByName = new Map<string, Set<string>>()
  for (const project of projects) {
    const hosts = hostsByName.get(project.name) ?? new Set<string>()
    hosts.add(projectHostName(project))
    hostsByName.set(project.name, hosts)
  }
  return new Set(projects.filter((project) => (hostsByName.get(project.name)?.size ?? 0) > 1).map((project) => project.id))
}

/** The project as the target of an operation (`operations.ts`), such as Remove Project. */
export function projectTarget(id: string): string {
  return `project:${id}`
}
