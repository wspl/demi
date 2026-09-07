import { Clock, Folder, House, Monitor } from '@lucide/vue'
import { createMemoryFileSource, dir, file, type MemoryDirectory } from '@demicodes/web-ui/files/memory-source'
import type { FileBrowserHost, FileBrowserPlaceGroup, FileBrowserSource } from '@demicodes/web-ui/files/types'

/**
 * Two devices' worth of directories for the file browser: a laptop with projects,
 * dotfiles, a locked folder and a cache with too many entries to see at once, and a
 * build box. A third device is offline. Nothing here reads a real disk.
 */

const day = (n: number, h = 10) => new Date(Date.UTC(2026, 8, n, h, 12)).toISOString()
const old = (m: number, d: number) => new Date(Date.UTC(2025, m - 1, d, 9, 30)).toISOString()

function project(files: Record<string, number>, modified: string): MemoryDirectory {
  return dir(
    {
      '.git': dir({ HEAD: file(21, modified), config: file(310, modified) }, { modifiedAt: modified }),
      src: dir(Object.fromEntries(Object.entries(files).map(([name, size]) => [name, file(size, modified)])), { modifiedAt: modified }),
      docs: dir({ 'README.md': file(4_210, modified), 'design.md': file(12_880, modified) }, { modifiedAt: modified }),
      'package.json': file(1_244, modified),
      'README.md': file(2_048, modified),
      '.gitignore': file(64, modified),
    },
    { modifiedAt: modified },
  )
}

function caches(): MemoryDirectory {
  const children: MemoryDirectory['children'] = {}
  for (let i = 1; i <= 64; i++) {
    const name = `cache-${String(i).padStart(3, '0')}.bin`
    children[name] = file(1024 * (i * 37 % 900) + i, old(((i * 5) % 12) + 1, ((i * 7) % 27) + 1))
  }
  return dir(children, { modifiedAt: old(12, 20) })
}

export function laptopTree(): MemoryDirectory {
  return dir({
    Applications: dir({}, { modifiedAt: old(11, 2) }),
    Users: dir({
      zan: dir(
        {
          Desktop: dir({ 'screenshot 2026-09-04.png': file(1_882_112, day(4, 16)), 'notes.txt': file(512, day(6)) }, { modifiedAt: day(6) }),
          Documents: dir({ 'Tax 2025.pdf': file(2_401_338, old(4, 12)), Invoices: dir({}, { modifiedAt: old(6, 1) }) }, { modifiedAt: old(6, 1) }),
          Downloads: dir({ 'bun-v1.3.14.zip': file(58_112_412, day(5)), 'demi-runner-darwin-arm64': file(21_404_010, day(3)) }, { modifiedAt: day(5) }),
          Projects: dir(
            {
              demi: project({ 'index.ts': 1_280, 'server.ts': 6_902, 'client.ts': 4_100 }, day(7, 9)),
              assetsfactory: project({ 'pipeline.ts': 9_210, 'thumbs.ts': 3_004 }, day(2)),
              wynk: project({ 'main.ts': 5_555 }, old(12, 30)),
              'a project with a very long directory name that will not fit in the address bar': project({ 'index.ts': 12 }, old(9, 9)),
            },
            { modifiedAt: day(7, 9) },
          ),
          Library: dir({ Caches: caches(), Preferences: dir({}, { modifiedAt: old(1, 1) }) }, { modifiedAt: old(12, 20) }),
          dotfiles: dir({ 'zshrc': file(3_311, day(1)), 'gitconfig': file(420, old(3, 3)), 'install.sh': file(1_012, old(3, 3)) }, { modifiedAt: day(1) }),
          '.config': dir({ demi: dir({ 'settings.json': file(802, day(6)) }, { modifiedAt: day(6) }) }, { modifiedAt: day(6) }),
          '.ssh': dir({}, { failure: { kind: 'permission', message: 'The runner cannot read /Users/zan/.ssh.' } }),
          '.zshrc': file(3_311, day(1)),
          '.gitconfig': file(420, old(3, 3)),
        },
        { modifiedAt: day(7, 9) },
      ),
    }),
    tmp: dir({}, { modifiedAt: day(7) }),
  })
}

export function buildBoxTree(): MemoryDirectory {
  return dir({
    home: dir({ build: dir({ '.bashrc': file(220, old(2, 2)), work: dir({}, { modifiedAt: day(5) }) }, { modifiedAt: day(5) }) }),
    srv: dir({ assetsfactory: project({ 'pipeline.ts': 9_210, 'thumbs.ts': 3_004 }, day(2)) }, { modifiedAt: day(2) }),
    var: dir({ log: dir({ 'assetsfactory.log': file(40_331_002, day(7, 8)) }, { modifiedAt: day(7, 8) }) }),
  })
}

export interface GalleryFileHost extends FileBrowserHost {
  source: FileBrowserSource
  places: FileBrowserPlaceGroup[]
}

export function createGalleryFileHosts(latencyMs = 250): GalleryFileHost[] {
  return [
    {
      id: 'mac',
      label: 'zan-mbp',
      online: true,
      source: createMemoryFileSource({ home: '/Users/zan', root: laptopTree(), latencyMs }),
      places: [
        { label: 'Quick access', places: [{ path: '/Users/zan', label: 'Home', icon: House }, { path: '/Users/zan/Desktop', icon: Folder }, { path: '/Users/zan/Projects', icon: Folder }] },
        { label: 'Recent', places: [{ path: '/Users/zan/Projects/demi', icon: Clock }, { path: '/Users/zan/dotfiles', icon: Clock }] },
      ],
    },
    {
      id: 'build',
      label: 'build-01',
      online: true,
      source: createMemoryFileSource({ home: '/home/build', root: buildBoxTree(), latencyMs }),
      places: [
        { label: 'Quick access', places: [{ path: '/home/build', label: 'Home', icon: House }] },
        { label: 'Recent', places: [{ path: '/srv/assetsfactory', icon: Clock }] },
      ],
    },
    {
      id: 'studio',
      label: 'studio',
      online: false,
      icon: undefined,
      source: createMemoryFileSource({ home: '/home/zan', root: dir({}), latencyMs, offline: true }),
      places: [{ label: 'Quick access', places: [{ path: '/home/zan', label: 'Home', icon: House }] }],
    },
  ]
}

export const cloudHost: FileBrowserHost = { id: 'cloud', label: 'Cloud', online: true, icon: Monitor }
