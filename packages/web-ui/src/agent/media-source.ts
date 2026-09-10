/** Backend transcripts display media through authenticated blob references. */
export interface BlobReferenceSource {
  type: 'ref'
  ref: string
  mediaType: string
  fileName?: string
}
