/**
 * An artifact a device's runner holds in its cache (`crates/runner-command-packages/src/cache.rs`): the package, the artifact's line and its version.
 */
export interface DeviceArtifact {
  package: string
  /** The artifact's line, such as `program` or `Chrome for Testing`. */
  name: string
  version: string
}

/**
 * An artifact a device's runner downloads into its cache now: what it is, as a
 * {@link DeviceArtifact} says, and how many of its `size` bytes arrived.
 */
export interface DeviceDownload extends DeviceArtifact, DownloadProgress {}

/** How far a download came: `received` of its `size` bytes. */
export interface DownloadProgress {
  received: number
  size: number
}

/** A Device as what it holds is read: its id, what its runner's cache holds and what it downloads. */
export interface HoldingDevice {
  id: string
  installed: readonly DeviceArtifact[]
  downloading: readonly DeviceDownload[]
}

/** What `device`, one of `devices`, holds of `packages`; nothing for a device not among them. */
export function deviceInstalled(
  devices: readonly HoldingDevice[],
  device: string,
  packages: readonly string[],
): DeviceArtifact[] {
  const installed = devices.find((entry) => entry.id === device)?.installed ?? []
  return installed.filter((artifact) => packages.includes(artifact.package))
}

/** What `device`, one of `devices`, downloads of `packages` now; nothing for a device not among them. */
export function deviceDownloading(
  devices: readonly HoldingDevice[],
  device: string,
  packages: readonly string[],
): DeviceDownload[] {
  const downloading = devices.find((entry) => entry.id === device)?.downloading ?? []
  return downloading.filter((download) => packages.includes(download.package))
}

/** How far `downloads` came together; null while there is none. */
export function downloadProgress(downloads: readonly DeviceDownload[]): DownloadProgress | null {
  if (downloads.length === 0) {
    return null
  }
  return downloads.reduce(
    (sum, download) => ({ received: sum.received + download.received, size: sum.size + download.size }),
    { received: 0, size: 0 },
  )
}
