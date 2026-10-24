import { hostInstalled, type HostArtifact } from '@demicodes/web-ui/devices/installed'
import type { PageState } from './lists'

/** What `hostId` holds of `packages`, a plugin's (`crates/runner-command-packages/src/cache.rs`). */
export function packageInstalled(
  state: PageState | null,
  hostId: string | null,
  packages: readonly string[],
): HostArtifact[] {
  return hostId === null ? [] : hostInstalled(state?.devices ?? [], hostId, packages)
}
