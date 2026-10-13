import { computed, type ComputedRef } from 'vue'
import { hostImage, type HostImage } from '../files/host-image'
import { useShowings } from '../files/showing'
import { messageImage } from './filePath'
import { messageImageTargets } from './render'
import type { MessageFiles } from './types'

/** The absolute paths of the Host images `content` names, in order. */
function hostImagePaths(content: string, files: MessageFiles | undefined): string[] {
  const paths: string[] = []
  for (const target of messageImageTargets(content)) {
    const image = messageImage(target, files)
    if (image && 'path' in image)
      paths.push(image.path)
  }
  return paths
}

/**
 * What shows of each Host image a message's `content` names, by its path
 * (`file-previews.md` § Files named in messages): each file's description
 * is shown from the conversation's kept files while the calling component
 * lives, so when the Host reports the file changed it is read again and its
 * image shows the new version, in this message and in every other that
 * names it.
 */
export function useHostImages(
  content: () => string,
  files: () => MessageFiles | undefined,
): ComputedRef<ReadonlyMap<string, HostImage>> {
  const contents = () => files()?.contents
  const shown = useShowings(contents, () => hostImagePaths(content(), files()), (kept, path) => kept.showDescription(path))
  return computed(() => new Map([...shown.value].map(([path, entry]) => [path, hostImage(entry, contents(), path)])))
}
