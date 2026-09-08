export { createRunnerHost, type RunnerHostOptions } from './host'
export { stdinStream, stdoutWriter, stderrWriter } from './stdio'
export { spawnTeed, readTail, type TeedSpawnHandle, type TeedSpawnParams } from './jobs'
export { connectWebSocket, connectUnix, listenUnix, type StreamSocket, type UnixListener, type WebSocketLink } from './net'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
export const msgpackEncode = msgpackCodec.encode
export const msgpackDecode = msgpackCodec.decode
// The process itself, for the runner and the command-mode entry: the only
// way they touch the txiki.js API.
export { argv, cwd, dropPrivileges, env, exit, fdNode, identity, onSignal, pid, version } from './runtime'
export { httpGet, httpPut, type HttpResponse } from './http'
