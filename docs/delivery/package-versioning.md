# Package versioning

Demi publishes four TypeScript packages to npm, and they share one version. The
Changesets configuration places them in a single fixed group, so any
release-worthy change bumps the whole group together and publishes every public
package in the group. One version keeps the set consistent: `web-ui` depends on
the other three, and a consumer who installs the same version of each gets
packages that were released together.

The Rust executables are versioned separately. They carry the Cargo workspace
version and are not npm packages or members of the Changesets group
([Rust executables](#rust-executables)).

## Release set

The fixed group contains the published packages:

- `@demicodes/web-ui`: the reusable Vue component library of the product and
  the gallery.
- `@demicodes/protocol`: the contract types and schemas generated from the Rust
  contract crates.
- `@demicodes/agent-client`: `AgentClient`, its WebSocket transport, and
  transcript patch application.
- `@demicodes/utils`: generic helpers for the browser packages.

The private `@demicodes/web` and `@demicodes/web-gallery` belong to the group
as well: they carry the shared version but are never published.

`@demicodes/protocol` has no hand-written source. A change to a Rust contract
type that the generator emits changes this package, so the changeset for that
change names it
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).

## Version selection

Every changeset declares the semantic impact of its change. Changesets selects
the highest required bump for the fixed group and applies the resulting version
to every package in the release set. Package manifests, changelogs, npm
artifacts, and Git tags therefore expose the same group version.

The repository release command publishes every group package whose shared
version is absent from the registry. It refreshes the lockfile, generates the
contract TypeScript from the Rust crates, builds the packages, validates packed
dependency ranges and export targets, publishes the tarballs, and creates one
package tag per artifact.

## Rust executables

The root `Cargo.toml` sets the workspace version under `[workspace.package]`,
and every crate inherits it. The runner, the command programs, the backend, and
the machine manager carry that version, and the backend and machine-manager
releases carry it too ([Packaging](builds-and-releases.md#packaging)). The
other two kinds of release use it differently:

- A command package's descriptor records it as the package version. A published
  version is immutable, so a release with different artifacts needs a new
  workspace version.
- A runner release is named by the hash of its contents rather than by the
  version.

The workspace version and the npm group version change independently: a
release of one does not bump the other.
