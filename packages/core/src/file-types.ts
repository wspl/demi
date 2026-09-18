// The file types the product previews, by extension (`file-previews.md`
// § Choosing a view). The backend serves a file under the media type this
// table gives it and shows in place only the types marked so; `web-ui` picks
// the viewer from the same type, so the two always agree.

interface PreviewType {
  mediaType: string
  extensions: readonly string[]
  /** Served as itself for the page to show; otherwise a download. */
  inPlace: boolean
}

const PREVIEW_TYPES: readonly PreviewType[] = [
  { mediaType: 'image/png', extensions: ['png'], inPlace: true },
  { mediaType: 'image/jpeg', extensions: ['jpg', 'jpeg'], inPlace: true },
  { mediaType: 'image/gif', extensions: ['gif'], inPlace: true },
  { mediaType: 'image/webp', extensions: ['webp'], inPlace: true },
  { mediaType: 'image/avif', extensions: ['avif'], inPlace: true },
  { mediaType: 'image/bmp', extensions: ['bmp'], inPlace: true },
  { mediaType: 'image/x-icon', extensions: ['ico'], inPlace: true },
  { mediaType: 'image/svg+xml', extensions: ['svg'], inPlace: true },
  { mediaType: 'video/mp4', extensions: ['mp4', 'm4v'], inPlace: true },
  { mediaType: 'video/webm', extensions: ['webm'], inPlace: true },
  { mediaType: 'video/quicktime', extensions: ['mov'], inPlace: true },
  { mediaType: 'audio/mpeg', extensions: ['mp3'], inPlace: true },
  { mediaType: 'audio/wav', extensions: ['wav'], inPlace: true },
  { mediaType: 'audio/ogg', extensions: ['ogg', 'oga', 'opus'], inPlace: true },
  { mediaType: 'audio/mp4', extensions: ['m4a'], inPlace: true },
  { mediaType: 'audio/aac', extensions: ['aac'], inPlace: true },
  { mediaType: 'audio/flac', extensions: ['flac'], inPlace: true },
  { mediaType: 'audio/webm', extensions: ['weba'], inPlace: true },
  { mediaType: 'application/pdf', extensions: ['pdf'], inPlace: true },
  // Rendered from its text, never served as itself.
  { mediaType: 'text/markdown', extensions: ['md', 'markdown'], inPlace: false },
]

const BY_EXTENSION = new Map(
  PREVIEW_TYPES.flatMap((type) => type.extensions.map((extension) => [extension, type.mediaType] as const)),
)

const IN_PLACE = new Set(PREVIEW_TYPES.filter((type) => type.inPlace).map((type) => type.mediaType))

/**
 * The media type the product knows a file by, from its extension whatever
 * its case; null for a file the table does not name.
 */
export function previewMediaType(path: string): string | null {
  // Either separator: a paired Windows device names its paths with `\`.
  const name = path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1)
  const dot = name.lastIndexOf('.')
  return dot > 0 ? BY_EXTENSION.get(name.slice(dot + 1).toLowerCase()) ?? null : null
}

/** Whether the page shows this media type in place instead of downloading it. */
export function showsInPlace(mediaType: string): boolean {
  return IN_PLACE.has(mediaType)
}
