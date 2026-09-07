import { Clock, House } from '@lucide/vue'
import { createMemoryFileSource, dir, file, type MemoryDirectory } from '@demicodes/web-ui/files/memory-source'
import type { FileBrowserHost, FileBrowserPlaceGroup, FileBrowserSource } from '@demicodes/web-ui/files/types'
import type { Device, Project } from './types'

/**
 * The directories the prototype's devices show in the file browser. Fixture trees,
 * one per device plus the Cloud workspace; nothing reads a real disk. A source is
 * built once per device so folders created in the browser stay for the session.
 */

const when = (d: number, h = 10) => new Date(Date.UTC(2026, 8, d, h)).toISOString()

function project(files: Record<string, number>): MemoryDirectory {
  return dir(
    {
      '.git': dir({ HEAD: file(21, when(1)) }, { modifiedAt: when(1) }),
      src: dir(Object.fromEntries(Object.entries(files).map(([name, size]) => [name, file(size, when(6))])), { modifiedAt: when(6) }),
      'README.md': file(2_048, when(2)),
      'package.json': file(1_244, when(2)),
    },
    { modifiedAt: when(6) },
  )
}

const trees: Record<string, () => MemoryDirectory> = {
  mac: () =>
    dir({
      Users: dir({
        zan: dir(
          {
            Desktop: dir({ 'notes.txt': file(512, when(6)) }, { modifiedAt: when(6) }),
            Documents: dir({}, { modifiedAt: when(3) }),
            Projects: dir(
              {
                demi: project({ 'index.ts': 1_280, 'server.ts': 6_902 }),
                notes: dir({ 'outline.md': file(3_100, when(5)), 'draft.md': file(12_400, when(7)) }, { modifiedAt: when(7) }),
              },
              { modifiedAt: when(7) },
            ),
            '.zshrc': file(3_311, when(1)),
          },
          { modifiedAt: when(7) },
        ),
      }),
    }),
  build: () =>
    dir({
      home: dir({ build: dir({ work: dir({}, { modifiedAt: when(5) }) }, { modifiedAt: when(5) }) }),
      srv: dir({ assetsfactory: project({ 'pipeline.ts': 9_210 }) }, { modifiedAt: when(2) }),
    }),
  cloud: () =>
    dir({
      home: dir({ demi: dir({ workspace: dir({}, { modifiedAt: when(7) }) }, { modifiedAt: when(7) }) }),
    }),
}

const sources = new Map<string, FileBrowserSource>()

/** The Cloud workspace's home, as the managed host lays it out. */
export const CLOUD_HOME = '/home/demi'

export function fileSourceFor(device: Device | null): FileBrowserSource {
  const id = device?.id ?? 'cloud'
  let source = sources.get(id)
  if (!source) {
    source = createMemoryFileSource({ home: device?.home ?? CLOUD_HOME, root: (trees[id] ?? trees['cloud']!)(), latencyMs: 200 })
    sources.set(id, source)
  }
  return source
}

/** Home first, then the workspaces already on that device. */
export function placesFor(device: Device | null, projects: Project[]): FileBrowserPlaceGroup[] {
  const deviceId = device?.id ?? 'cloud'
  const recent = projects.filter((item) => item.deviceId === deviceId).map((item) => ({ path: item.path, label: item.name, icon: Clock }))
  return [
    { label: 'Quick access', places: [{ path: device?.home ?? CLOUD_HOME, label: 'Home', icon: House }] },
    ...(recent.length ? [{ label: 'Workspaces', places: recent }] : []),
  ]
}

export function browserHosts(devices: Device[], includeCloud: boolean): FileBrowserHost[] {
  return [
    ...devices.map((device) => ({ id: device.id, label: device.name, online: device.online })),
    ...(includeCloud ? [{ id: 'cloud', label: 'Cloud', online: true }] : []),
  ]
}
