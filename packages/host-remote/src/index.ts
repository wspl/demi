// The backend's end of a runner: the Host the agent holds for every user
// host and managed host, and its shell environment over jobs (`runner.md`).
export {
  RemoteHost,
  RemoteGitError,
  RemoteLogError,
  RemoteNetError,
  RemoteServiceError,
  type RemoteGit,
  type RemoteLog,
  type RemoteNet,
  type RemoteServices,
  type RemoteServiceStream,
  type RemoteHostOptions,
  type RemoteJob,
  type RemoteJobExit
} from './remote-host'
export {
  PipeBroker,
  devicePipes,
  type DeviceEnd,
  type HostPipes,
  type Pipe,
  type PipeWriter
} from './pipes'
export {
  RemoteShellEnvironment,
  type RemoteShellEnvironmentOptions
} from './remote-shell-environment'

export {
  createRemoteShellEnvironmentFactory,
  type RemoteShellEnvironmentFactoryOptions,
  type RemoteShellEnvironmentContext,
  type RemoteCommandCatalog,
} from './shell-environment-factory'
