import { hostDownloading, hostInstalled, type HostArtifact, type HostDownload } from '@demicodes/web-ui/devices/installed'
import type { PageState } from './lists'

/** What `hostId` holds of `packages`, a plugin's (`crates/runner-command-packages/src/cache.rs`). */
export function packageInstalled(
  state: PageState | null,
  hostId: string | null,
  packages: readonly string[],
): HostArtifact[] {
  return hostId === null ? [] : hostInstalled(state?.devices ?? [], hostId, packages)
}

/** What `hostId` downloads of `packages`, a plugin's, now. */
export function packageDownloading(
  state: PageState | null,
  hostId: string | null,
  packages: readonly string[],
): HostDownload[] {
  return hostId === null ? [] : hostDownloading(state?.devices ?? [], hostId, packages)
}
