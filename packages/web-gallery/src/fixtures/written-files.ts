import { pageMessageFiles, type PageMessageFiles } from '@demicodes/web-ui/agent/comments/page-files'
import { galleryUploads } from './upload-sweep'

/**
 * The files of a specimen's comments and side questions, which upload at a
 * pace slow enough to watch, as the composer's do, and paste as the
 * product's do.
 */
export function galleryMessageFiles(): PageMessageFiles {
  return pageMessageFiles(galleryUploads())
}
