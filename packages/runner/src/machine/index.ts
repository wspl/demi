export { createRunnerHost, TINYJS_ABI, type RunnerHostOptions } from './host'
export { openPipe, readHandle, stdinStream, stdoutWriter, stderrWriter, writerFor } from './stdio'
export { spawnTeed, readTail, type TeedSpawnHandle, type TeedSpawnParams } from './jobs'
export { connectWebSocket, connectUnix, listenUnix, type StreamSocket, type UnixListener, type WebSocketLink } from './net'
export { msgpackDecode, msgpackEncode } from 'tinyjs:bytes'
// The process itself, for the runner and the command-mode entry: the only
// way they touch the tinyjs API.
export { argv, cwd, env, exit, fdNode, identity, onSignal, openHandles, pid, version } from 'tinyjs:runtime'
export { httpGet, httpPut, type HttpResponse } from './http'
