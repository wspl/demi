import type { ShellCommandSnapshot, StreamArtifact } from '@demicodes/shell'

/**
 * Keep one interleaved output copy for restored terminal rendering, plus artifact references and
 * command audit data. The model-facing output already owns the preview and media payloads, so the
 * raw stream bodies, duplicate merged text/tails, and asset base64 do not belong in metadata.
 */
export function compactShellToolMetadata(result: ShellCommandSnapshot): unknown {
  const base = {
    status: result.status,
    shellId: result.shellId,
    commandId: result.commandId,
    stdout: artifactMetadata(result.stdout),
    stderr: artifactMetadata(result.stderr),
    output: {
      path: result.output.path,
      offset: result.output.offset,
      chunks: result.output.chunks,
      bytes: result.output.bytes,
      truncated: result.output.truncated,
    },
    runningMs: result.runningMs,
    idleMs: result.idleMs,
  }
  if (result.status !== 'exited') return base
  return {
    ...base,
    exitCode: result.exitCode,
    audit: result.audit,
    ...(result.commandMetadata ? { commandMetadata: result.commandMetadata } : {}),
  }
}

function artifactMetadata(artifact: StreamArtifact): Omit<StreamArtifact, 'delta' | 'tail'> {
  return {
    path: artifact.path,
    offset: artifact.offset,
    bytes: artifact.bytes,
    truncated: artifact.truncated,
  }
}
