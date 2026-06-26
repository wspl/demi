import { decodeUtf8, errorMessage } from '@demi/utils'
import type { Host } from '@demi/shell'
import type { AgentReferenceResolveContext } from '@demi/agent'
import type { UserContentBlock } from '@demi/core'

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
  const pathError = pathValidationError(path)
  if (pathError) throw new Error(pathError)
  let stdout: string
  try {
    stdout = decodeUtf8(await host.fs.readFile(path, { cwd }))
  } catch (error) {
    throw new Error(`Failed to resolve file reference "${reference}": ${errorMessage(error)}`)
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

function pathValidationError(path: string): string | null {
  if (path.includes('\0')) return `File reference contains NUL byte: ${path}`
  return null
}
