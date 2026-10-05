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
