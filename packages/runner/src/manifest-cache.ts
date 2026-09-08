// The manifest cache and the root-command symlinks (`commands.md` § Root
// commands on a target): `commands/<hash>/` per manifest, `commands/current`
// pointing at the one in force, and immutable per-manifest root aliases to the C client.
import { parseManifest, writeManifestDirectory, type Manifest } from '@demicodes/command-loader'
import type { HostFileSystem } from '@demicodes/shell'
import { isFileNotFoundError } from '@demicodes/utils'

export class ManifestCache {
  constructor(
    private readonly fs: HostFileSystem,
    private readonly commandsDir: string,
    /** The native client every root symlink points at. */
    private readonly executable: string,
  ) {}

  binDirectory(manifest: Manifest): string {
    return `${this.commandsDir}/${manifest.hash}/bin`
  }

  /** The manifest in force, if any. */
  async current(): Promise<Manifest | null> {
    try {
      const bytes = await this.fs.readFile(`${this.commandsDir}/current/manifest.json`)
      return parseManifest(JSON.parse(new TextDecoder().decode(bytes)))
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
    const binDir = this.binDirectory(manifest)
    await this.fs.mkdir(binDir, { recursive: true })
    for (const root of Object.keys(manifest.roots)) {
      await replaceSymlink(this.fs, this.executable, `${binDir}/${root}`)
    }
    await replaceSymlink(this.fs, manifest.hash, `${this.commandsDir}/current`)
    return manifest
  }
}

async function replaceSymlink(fs: HostFileSystem, target: string, path: string): Promise<void> {
  const temp = `${path}.${crypto.randomUUID()}`
  await fs.symlink(target, temp)
  await fs.mv(temp, path)
}
