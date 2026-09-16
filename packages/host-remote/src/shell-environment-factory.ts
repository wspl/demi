import { nativePackageSchema, type ArtifactResolver, type NativePackage } from '@demicodes/command-protocol'
import { buildManifest, type Manifest } from '@demicodes/command-loader'
import type { CommandRegistry, CommandStorage, Host, ShellEnvironmentOptions } from '@demicodes/shell'
import { RemoteHost } from './remote-host'
import { RemoteShellEnvironment, type RemoteShellEnvironmentOptions } from './remote-shell-environment'

export interface RemoteCommandCatalog {
  manifest: Manifest
  resolveArtifact: ArtifactResolver
}

export interface RemoteShellEnvironmentFactoryOptions {
  packages: readonly NativePackage[]
  resolveArtifact: ArtifactResolver
}

/** Structural context keeps the execution adapter independent of the agent package. */
export interface RemoteShellEnvironmentContext {
  rootSessionId: string
  agentSessionId: string
  host: Host
  commands: Pick<CommandRegistry, 'list'>
  commandStorage(signal?: AbortSignal): CommandStorage
  shell: ShellEnvironmentOptions
  retainEdits?: RemoteShellEnvironmentOptions['retainEdits']
  runJob?: RemoteShellEnvironmentOptions['runJob']
}

export function createRemoteShellEnvironmentFactory(options: RemoteShellEnvironmentFactoryOptions) {
  const packages = options.packages.map(value => nativePackageSchema.parse(value))
  const ids = new Set<string>()
  for (const descriptor of packages) {
    if (ids.has(descriptor.id))
      throw new Error(`Duplicate native package id: ${descriptor.id}`)
    ids.add(descriptor.id)
  }
  return async (context: RemoteShellEnvironmentContext): Promise<RemoteShellEnvironment> => {
    if (!(context.host instanceof RemoteHost))
      throw new Error('Remote shell environment requires a RemoteHost')
    const manifest = await buildManifest(context.commands.list(), { packages })
    return new RemoteShellEnvironment({
      ...context.shell,
      host: context.host,
      conversation: context.rootSessionId,
      node: context.agentSessionId,
      commandStorage: context.commandStorage,
      retainEdits: context.retainEdits,
      runJob: context.runJob,
      commands: { manifest, resolveArtifact: options.resolveArtifact },
    })
  }
}
