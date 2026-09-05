// The runner's end of the protocol, exported for tests that join it to a
// `RemoteHost` without a socket (`@demicodes/runner/serve`).
export { HostRpcServer } from './host-rpc-server'
export { JobTable, JOB_CWD_FILE_VAR, JOB_STDIN_FD, JOB_STDIN_FD_VAR, wrapScript, type JobSpawnHandle, type JobSpawnParams, type JobTableOptions } from './jobs'
