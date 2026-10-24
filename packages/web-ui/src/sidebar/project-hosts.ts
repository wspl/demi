import { t } from './locales/text'
import type { SidebarDevice, SidebarProject } from './types'

/** What a host is called where the sidebar names it: the Cloud, a device by its name, or a removed one. */
export function projectHostName(project: SidebarProject): string {
  if (project.hostKind === 'cloud') {
    return t('device.cloud')
  }
  return deviceName(project.device)
}

function deviceName(device: SidebarDevice): string {
  return device.kind === 'paired' ? device.name : t('device.removed')
}

/** What a project's row says after its name to tell it from a namesake, and the whole of where it is, its tooltip. */
export interface ProjectQualifier {
  text: string
  hint: string
}

/**
 * What each project's row says after its name, for the projects that share their name with another:
 *
 * - On another host, the host's name, since the mark alone cannot tell them apart.
 * - On the same host, the shortest end of its parent folders that no namesake there shares, as an
 *   editor tells two tabs of one file name apart: `work` for `~/work/app` beside `~/old/app`, and
 *   `a/src` where `src` alone would not do; `/` for a folder at the root.
 *
 * Both together are joined by a middle dot. A project of a name of its own is not in the map.
 */
export function projectQualifiers(projects: readonly SidebarProject[]): Map<string, ProjectQualifier> {
  const byName = new Map<string, SidebarProject[]>()
  for (const project of projects) {
    byName.set(project.name, [...(byName.get(project.name) ?? []), project])
  }
  const qualifiers = new Map<string, ProjectQualifier>()
  for (const namesakes of byName.values()) {
    if (namesakes.length < 2) {
      continue
    }
    const hosts = new Set(namesakes.map(projectHostName))
    for (const project of namesakes) {
      const host = projectHostName(project)
      const here = namesakes.filter((other) => projectHostName(other) === host)
      const words = [
        ...(hosts.size > 1 ? [host] : []),
        ...(here.length > 1 ? [distinctParent(project, here.filter((other) => other !== project))] : []),
      ]
      qualifiers.set(project.id, { text: words.join(' · '), hint: hosts.size > 1 ? `${host}: ${project.path}` : project.path })
    }
  }
  return qualifiers
}

/** The folders `path` is in, outermost first, and the separator it is written with. */
function parentFolders(path: string): { folders: string[]; separator: string } {
  const separator = path.includes('\\') && !path.includes('/') ? '\\' : '/'
  const folders = path.split(separator).filter(Boolean)
  folders.pop()
  return { folders, separator }
}

/** The shortest end of `project`'s parent folders that none of `others` ends with. */
function distinctParent(project: SidebarProject, others: readonly SidebarProject[]): string {
  const { folders, separator } = parentFolders(project.path)
  const theirs = others.map((other) => parentFolders(other.path).folders)
  for (let length = 1; length <= folders.length; length++) {
    const end = folders.slice(-length).join(separator)
    if (theirs.every((folders) => folders.slice(-length).join(separator) !== end)) {
      return end
    }
  }
  return folders.join(separator) || separator
}

/** The project as the target of an operation (`operations.ts`), such as Remove Project. */
export function projectTarget(id: string): string {
  return `project:${id}`
}
