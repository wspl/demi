export { createRunnerHost, TINYJS_ABI, type RunnerHostOptions } from './host'
export { readHandle, stdinStream, stdoutWriter, stderrWriter, writerFor } from './stdio'
// The process itself, for the runner and the command-mode entry: the only
// way they touch the tinyjs API.
export { argv, cwd, env, exit, identity, onSignal, openHandles, version } from 'tinyjs:runtime'
