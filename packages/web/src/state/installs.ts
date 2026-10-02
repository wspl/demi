import { hostInstalls, type HostInstall, type InstallScope } from '@demicodes/web-ui/devices/installs'
import type { CatalogProvider, ProductState } from '../api/generated/web-api'

/*
 * Which installs each place of the page shows (`native-runtime.md`
 * § Installation progress), read from the devices of the product state.
 */

/**
 * The package of the CLI that `providerId`'s requests start on the Cloud, by
 * the catalog; null when its requests are HTTP or the catalog lacks it.
 */
export function cliPackageOf(catalog: readonly CatalogProvider[], providerId: string): string | null {
  return catalog.find((provider) => provider.providerId === providerId)?.cliPackage ?? null
}

/**
 * What a conversation's Hosts install: everything its Host `hostId`
 * installs, and the Cloud's installs of `cliPackage`, the package of the
 * CLI its provider starts there, when it has one.
 */
export function conversationInstalls(
  state: ProductState | null,
  hostId: string | null,
  cliPackage: string | null,
): HostInstall[] {
  const scopes: InstallScope[] = []
  if (hostId !== null) {
    scopes.push({ host: hostId })
  }
  const cloud = cloudOf(state)
  if (cloud !== null && cliPackage !== null) {
    scopes.push({ host: cloud, packages: [cliPackage] })
  }
  return hostInstalls(state?.devices ?? [], scopes)
}

/** What the Cloud installs of `cliPackage`, the package of a provider's CLI. */
export function cliInstalls(state: ProductState | null, cliPackage: string): HostInstall[] {
  const cloud = cloudOf(state)
  return cloud === null ? [] : hostInstalls(state?.devices ?? [], [{ host: cloud, packages: [cliPackage] }])
}

/** What `hostId` installs of `packages`, a plugin's. */
export function packageInstalls(
  state: ProductState | null,
  hostId: string | null,
  packages: readonly string[],
): HostInstall[] {
  return hostId === null ? [] : hostInstalls(state?.devices ?? [], [{ host: hostId, packages }])
}

/** Everything the Cloud's runner installs now. */
export function cloudInstalls(state: ProductState | null): HostInstall[] {
  const cloud = cloudOf(state)
  return cloud === null ? [] : hostInstalls(state?.devices ?? [], [{ host: cloud }])
}

/** The Cloud's device id; null before the user has a Cloud or a snapshot. */
function cloudOf(state: ProductState | null): string | null {
  return state?.cloud.device?.id ?? null
}
