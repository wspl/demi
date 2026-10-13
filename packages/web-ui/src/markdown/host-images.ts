import { computed, type ComputedRef } from 'vue'
import { hostImage, type HostImage } from '../files/host-image'
import { useShowings } from '../files/showing'
import type { FileContents } from '../files/types'
import { messageImage } from './filePath'
import { messageImageTargets } from './render'
import type { MessageFiles } from './types'

/** The absolute paths of the Host images `content` names, in order. */
export function hostImagePaths(content: string, files: MessageFiles | undefined): string[] {
  const paths: string[] = []
  for (const target of messageImageTargets(content)) {
    const image = messageImage(target, files)
    if (image && 'path' in image)
      paths.push(image.path)
  }
  return paths
}

/**
 * What shows of each Host image at `paths`, by its path
 * (`file-previews.md` § Files named in messages): each file's description
 * is shown from the kept files `contents` gives while the calling component
 * lives, so when the Host reports the file changed it is read again and its
 * image shows the new version, wherever it is named. The one owner of how a
 * Host image follows its file, for a message and for a Markdown document.
 */
export function useHostImagesAt(
  contents: () => FileContents | undefined,
  paths: () => string[],
): ComputedRef<ReadonlyMap<string, HostImage>> {
  const shown = useShowings(contents, paths, (kept, path) => kept.showDescription(path))
  return computed(() => new Map([...shown.value].map(([path, entry]) => [path, hostImage(entry, contents(), path)])))
}
