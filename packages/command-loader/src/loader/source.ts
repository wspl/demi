import type { HostFileSystem } from '@demicodes/shell'
import { decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import { parseManifest, type Manifest } from '../manifest/schema'

/** Where a loader gets its manifest: in memory, a directory, a socket, a URL. */
export interface ManifestSource {
  manifest(): Promise<Manifest>
  /**
   * Where a `runtime` module lives as a file, by its hash. An embedder whose
   * runtime imports only files (tinyjs) loads modules from here; absent, the
   * loader imports each module from its text.
   */
  modulePath?(hash: string): string
}

export function inMemorySource(manifest: Manifest): ManifestSource {
  return { manifest: async () => manifest }
}

/**
 * A manifest kept as files — the runner's cache on a target, or a directory
 * an embedder configured: `manifest.json` and one `modules/<hash>.mjs` per
 * `runtime` module, the layout `writeManifestDirectory` produces.
 */
export function directorySource(dir: string, fs: HostFileSystem): ManifestSource {
  return {
    manifest: async () => parseManifest(JSON.parse(decodeUtf8(await fs.readFile(`${dir}/manifest.json`)))),
    modulePath: (hash) => `${dir}/modules/${hash}.mjs`,
  }
}

/** Materializes a manifest in the layout `directorySource` reads. */
export async function writeManifestDirectory(manifest: Manifest, dir: string, fs: HostFileSystem): Promise<void> {
  await fs.mkdir(`${dir}/modules`, { recursive: true })
  await fs.writeFile(`${dir}/manifest.json`, encodeUtf8(JSON.stringify(manifest)))
  for (const [hash, javascript] of Object.entries(manifest.modules)) {
    await fs.writeFile(`${dir}/modules/${hash}.mjs`, encodeUtf8(javascript))
  }
}
