import { hostInstalled, type HostArtifact } from '@demicodes/web-ui/devices/installed'
import type { ProductState } from '../api/generated/web-api'

/** What `hostId` holds of `packages`, a plugin's (`native-runtime.md` § Installed artifacts). */
export function packageInstalled(
  state: ProductState | null,
  hostId: string | null,
  packages: readonly string[],
): HostArtifact[] {
  return hostId === null ? [] : hostInstalled(state?.devices ?? [], hostId, packages)
}
