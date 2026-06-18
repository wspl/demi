import type { AgentReferenceResolveContext } from '@demi/base-agent'
import type { Host } from '@demi/shell'
import type { UserContentBlock } from '@demi/core'
import { concatBytes, decodeUtf8, isPathInside, resolvePath } from './platform'

export function createFileReferenceResolver<State>(host: Host) {
  return async (
    ctx: AgentReferenceResolveContext<State>,
    content: UserContentBlock[],
  ): Promise<UserContentBlock[]> => {
    const resolved: UserContentBlock[] = []
    for (const block of content) {
      if (block.type !== 'reference') {
        resolved.push(block)
        continue
      }
      resolved.push(await resolveFileReference(host, ctx.cwd, block.reference))
    }
    return resolved
  }
}

async function resolveFileReference(host: Host, cwd: string, reference: string): Promise<UserContentBlock> {
  const path = parseFileReference(reference)
  const pathError = workspacePathError(host, cwd, path)
  if (pathError) throw new Error(pathError)
  const handle = await host.spawn({ command: 'cat', args: [path], cwd })
  const [stdout, stderr, exit] = await Promise.all([
    collectText(handle.stdout),
    collectText(handle.stderr),
    handle.wait(),
  ])
  if (exit.exitCode !== 0) {
    throw new Error(`Failed to resolve file reference "${reference}": ${stderr.trim() || `exit ${exit.exitCode ?? 1}`}`)
  }

  return {
    type: 'text',
    text: `<file path="${path}">\n${stdout}\n</file>`,
  }
}

function parseFileReference(reference: string): string {
  const trimmed = reference.trim()
  if (!trimmed) throw new Error('Empty file reference')
  if (trimmed.startsWith('file://') || trimmed.startsWith('file:/')) return decodeFileReferencePath(new URL(trimmed).pathname)
  if (trimmed.startsWith('file:')) return decodeFileReferencePath(trimmed.slice('file:'.length))
  return trimmed
}

function decodeFileReferencePath(path: string): string {
  try {
    return decodeURIComponent(path)
  } catch {
    throw new Error(`Invalid file reference URL encoding: ${path}`)
  }
}

function workspacePathError(host: Host, cwd: string, path: string): string | null {
  if (path.includes('\0')) return `File reference contains NUL byte: ${path}`
  const target = resolvePath(cwd, path)
  if (isPathInside(host.root, target)) return null
  return `File reference escapes workspace: ${path}`
}

async function collectText(iterable: AsyncIterable<Uint8Array>): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of iterable) chunks.push(chunk)
  return decodeUtf8(concatBytes(chunks))
}
