// The manifest cache and the root-command symlinks (`commands.md` § Root
// commands on a target): `commands/<hash>/` per manifest, `commands/current`
// pointing at the one in force, `bin/<root>` → the packed binary.
import { parseManifest, writeManifestDirectory, type Manifest } from '@demicodes/command-loader'
import type { HostFileSystem } from '@demicodes/shell'
import { isFileNotFoundError } from '@demicodes/utils'

export class ManifestCache {
  constructor(
    private readonly fs: HostFileSystem,
    private readonly commandsDir: string,
    private readonly binDir: string,
    /** The packed binary every root symlink points at. */
    private readonly executable: string,
  ) {}

  /** The manifest in force, if any. */
  async current(): Promise<Manifest | null> {
    try {
      return parseManifest(JSON.parse(new TextDecoder().decode(await this.fs.readFile(`${this.commandsDir}/current/manifest.json`))))
    } catch (error) {
      if (isFileNotFoundError(error)) return null
      throw error
    }
  }

  /** Stores a manifest received from the backend and points `current` and the root symlinks at it. */
  async install(value: unknown): Promise<Manifest> {
    const manifest = parseManifest(value)
    const dir = `${this.commandsDir}/${manifest.hash}`
    if (!(await this.fs.exists(`${dir}/manifest.json`))) await writeManifestDirectory(manifest, dir, this.fs)
    await replaceSymlink(this.fs, manifest.hash, `${this.commandsDir}/current`)
    await this.fs.mkdir(this.binDir, { recursive: true })
    const roots = new Set(Object.keys(manifest.roots))
    for (const name of await this.fs.readdir(this.binDir)) {
      if (!roots.has(name)) await this.fs.rm(`${this.binDir}/${name}`, { force: true })
    }
    for (const root of roots) await replaceSymlink(this.fs, this.executable, `${this.binDir}/${root}`)
    return manifest
  }
}

async function replaceSymlink(fs: HostFileSystem, target: string, path: string): Promise<void> {
  const temp = `${path}.${crypto.randomUUID()}`
  await fs.symlink(target, temp)
  await fs.mv(temp, path)
}
