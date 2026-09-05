import type { UserContentBlock } from '@demicodes/core'
import { sniffModelMediaType } from '@demicodes/core'

export async function fileToUserContent(file: File): Promise<UserContentBlock> {
  const bytes = new Uint8Array(await file.arrayBuffer())
  const sniffed = sniffModelMediaType(bytes)
  if (sniffed?.kind === 'image') {
    return { type: 'image', source: { type: 'binary', data: bytes, mediaType: sniffed.mediaType } }
  }
  if (sniffed?.kind === 'video') {
    return { type: 'video', source: { type: 'binary', data: bytes, mediaType: sniffed.mediaType } }
  }
  return {
    type: 'document',
    source: {
      data: bytes,
      mediaType: file.type || 'application/octet-stream',
      fileName: file.name,
    },
  }
}

export function fileMatchesAcceptedExtensions(file: File, acceptedExtensions: readonly string[]): boolean {
  if (acceptedExtensions.length === 0) return true
  const ext = file.name.split('.').pop()?.toLowerCase()
  if (!ext) return false
  if (acceptedExtensions.includes(ext)) return true
  return ext === 'jpeg' && acceptedExtensions.includes('jpg')
}

/** Splits a drop or paste into the files the model accepts and the ones it does not. */
export function partitionAcceptedFiles(
  files: readonly File[],
  acceptedExtensions: readonly string[],
): { accepted: File[]; rejected: File[] } {
  const accepted: File[] = []
  const rejected: File[] = []
  for (const file of files) {
    (fileMatchesAcceptedExtensions(file, acceptedExtensions) ? accepted : rejected).push(file)
  }
  return { accepted, rejected }
}

export function acceptAttribute(acceptedExtensions: readonly string[]): string | undefined {
  if (acceptedExtensions.length === 0) return undefined
  return acceptedExtensions.map((ext) => `.${ext}`).join(',')
}

export function filePreviewUrl(file: File): string | undefined {
  if (file.type.startsWith('image/')) return URL.createObjectURL(file)
  if (file.type === '' && /\.(png|jpe?g|gif|webp|bmp)$/i.test(file.name)) {
    return URL.createObjectURL(file)
  }
  return undefined
}

export function dataTransferFiles(transfer: DataTransfer): File[] {
  return [...transfer.files]
}

export function transferHasFiles(transfer: DataTransfer | null | undefined): boolean {
  return transfer?.types.includes('Files') === true
}
