/**
 * An artifact a Host's runner is installing now (`native-runtime.md`
 * § Installation progress): the package, the artifact's name and version,
 * the phase, and the bytes downloaded of the artifact's size.
 */
export interface HostInstall {
  package: string
  /** The artifact's line, such as `program` or `Chrome for Testing`. */
  name: string
  version: string
  phase: 'download' | 'unpack'
  done: number
  total: number
}

/**
 * An artifact a Host's runner holds in its cache (`native-runtime.md`
 * § Installed artifacts): the package, the artifact's line and its version.
 */
export interface HostArtifact {
  package: string
  /** The artifact's line, such as `program` or `Chrome for Testing`. */
  name: string
  version: string
}

/** A Host as its installs are read: its id and what its runner installs now. */
export interface InstallingHost {
  id: string
  installs: readonly HostInstall[]
}

/** Which installs of one Host a place shows: all of them, or those of some packages. */
export interface InstallScope {
  host: string
  packages?: readonly string[]
}

/**
 * The installs `scopes` select from `hosts`, in the scopes' order, each once
 * however many scopes select it: a conversation on the Cloud whose provider
 * also runs there selects the Cloud twice.
 */
export function hostInstalls(
  hosts: readonly InstallingHost[],
  scopes: readonly InstallScope[],
): HostInstall[] {
  const selected = new Set<HostInstall>()
  for (const scope of scopes) {
    const host = hosts.find((entry) => entry.id === scope.host)
    for (const install of host?.installs ?? []) {
      if (!scope.packages || scope.packages.includes(install.package)) {
        selected.add(install)
      }
    }
  }
  return [...selected]
}

/** A Host as what it holds is read: its id and what its runner's cache holds. */
export interface HoldingHost {
  id: string
  installed: readonly HostArtifact[]
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
