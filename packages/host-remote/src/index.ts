// The backend's end of a runner: the Host the agent holds for every user
// host and managed host, and its shell environment over jobs (`runner.md`).
export { RemoteHost, type RemoteHostOptions, type RemoteJob, type RemoteJobExit } from './remote-host'
export { RemoteShellEnvironment, type RemoteShellEnvironmentOptions } from './remote-shell-environment'
