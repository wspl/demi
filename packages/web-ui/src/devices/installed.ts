/**
 * An artifact a Host's runner holds in its cache (`crates/runner-command-packages/src/cache.rs`): the package, the artifact's line and its version.
 */
export interface HostArtifact {
  package: string
  /** The artifact's line, such as `program` or `Chrome for Testing`. */
  name: string
  version: string
}

/**
 * An artifact a Host's runner downloads into its cache now: what it is, as a
 * {@link HostArtifact} says, and how many of its `size` bytes arrived.
 */
export interface HostDownload extends HostArtifact, DownloadProgress {}

/** How far a download came: `received` of its `size` bytes. */
export interface DownloadProgress {
  received: number
  size: number
}

/** A Host as what it holds is read: its id, what its runner's cache holds and what it downloads. */
export interface HoldingHost {
  id: string
  installed: readonly HostArtifact[]
  downloading: readonly HostDownload[]
}

/** What `host`, one of `hosts`, holds of `packages`; nothing for a Host not among them. */
export function hostInstalled(
  hosts: readonly HoldingHost[],
  host: string,
  packages: readonly string[],
): HostArtifact[] {
  const installed = hosts.find((entry) => entry.id === host)?.installed ?? []
  return installed.filter((artifact) => packages.includes(artifact.package))
}

/** What `host`, one of `hosts`, downloads of `packages` now; nothing for a Host not among them. */
export function hostDownloading(
  hosts: readonly HoldingHost[],
  host: string,
  packages: readonly string[],
): HostDownload[] {
  const downloading = hosts.find((entry) => entry.id === host)?.downloading ?? []
  return downloading.filter((download) => packages.includes(download.package))
}

/** How far `downloads` came together; null while there is none. */
export function downloadProgress(downloads: readonly HostDownload[]): DownloadProgress | null {
  if (downloads.length === 0) {
    return null
  }
  return downloads.reduce(
    (sum, download) => ({ received: sum.received + download.received, size: sum.size + download.size }),
    { received: 0, size: 0 },
  )
}
