import { nextTick, watch } from 'vue'
import { HOST_FILE_ATTRIBUTE, HOST_SRCSET_ATTRIBUTE, hostSrcsetCandidates, srcsetText } from '../markdown/document'
import { useHostImagesAt } from '../markdown/host-images'
import { imagePlaceholderHtml } from '../markdown/image-placeholder'
import type { FileContents } from './types'

/**
 * The images of a Markdown document the page shows (`file-previews.md`
 * § Markdown, § Files named in messages), in the element `root` holds the
 * rendering `html` in.
 *
 * Each Host image of the document (`hostFiles` are their paths) is given the
 * address of its file's version the page keeps, where it stands, so a new
 * version loads in place and nothing else of the document is rendered
 * again; one whose file is still being read, or is not there, shows its alt
 * text in the placeholder box, never as an image without an address, which
 * the web browser draws as a broken picture. A `srcset` that names Host files (`HOST_SRCSET_ATTRIBUTE`) is
 * written once each of its Host files is read, each candidate at the address
 * of its file's version; a candidate whose file is not there drops out. The
 * image of a `<picture>` takes its address only once each of the picture's
 * sets is written, and keeps its placeholder until then, so the web browser
 * chooses among the sources once and fetches only the candidate it chooses:
 * a dark theme does not also fetch the light logo the image falls back to.
 *
 * An image whose bytes do not load, as while the Host is away, or whose file
 * is not there, shows its alt text in a quiet box in its place, as a web
 * browser shows a missing image's description, instead of its broken picture
 * glyph. Each box keeps its image, which tries again when the document is
 * read again (`reload`) and, for a Host image, when its file has a version it
 * has not tried.
 */
export function useDocumentImages(options: {
  root: () => HTMLElement | undefined
  html: () => string
  hostFiles: () => string[]
  contents: () => FileContents | undefined
}): { onError(event: Event): void; reload(): void } {
  const { html } = options
  const hostImages = useHostImagesAt(options.contents, options.hostFiles)
  /** Each placeholder shown, with its image. */
  const failed = new Map<HTMLElement, HTMLImageElement>()
  /**
   * The placeholders of images whose bytes did not load, as against those
   * still waiting for their file or their picture's sources: only these
   * wait for a version they have not tried.
   */
  const unloaded = new Set<HTMLElement>()

  /** Shows `image`'s alt text in its place. */
  function fail(image: HTMLImageElement): HTMLElement | null {
    const made = document.createElement('template')
    made.innerHTML = imagePlaceholderHtml(image.alt)
    const placeholder = made.content.firstElementChild
    if (!(placeholder instanceof HTMLElement))
      return null
    failed.set(placeholder, image)
    image.replaceWith(placeholder)
    return placeholder
  }

  function onError(event: Event): void {
    if (!(event.target instanceof HTMLImageElement))
      return
    const placeholder = fail(event.target)
    if (placeholder)
      unloaded.add(placeholder)
  }

  function reload(): void {
    for (const [placeholder, image] of failed) {
      // A Host image whose file is not there has no address to try again.
      if (!image.hasAttribute('src'))
        continue
      placeholder.replaceWith(image)
      // Setting the address again loads it again.
      image.src = image.src
      failed.delete(placeholder)
      unloaded.delete(placeholder)
    }
  }

  // A new rendering brings its own images.
  watch(html, () => {
    failed.clear()
    unloaded.clear()
  })

  async function address(): Promise<void> {
    // The rendering reaches the page first.
    await nextTick()
    const root = options.root()
    // The pictures with a set whose files are still being read.
    const choosing = new Set<Element>()
    for (const element of root?.querySelectorAll(`[${HOST_SRCSET_ATTRIBUTE}]`) ?? []) {
      const picture = element.closest('picture')
      if (!setSrcset(element) && picture)
        choosing.add(picture)
    }
    const placeholders = new Map([...failed].map(([placeholder, image]) => [image, placeholder]))
    const images = [...root?.querySelectorAll<HTMLImageElement>(`img[${HOST_FILE_ATTRIBUTE}]`) ?? [], ...placeholders.keys()]
    for (const image of images) {
      const shown = hostImages.value.get(image.getAttribute(HOST_FILE_ATTRIBUTE) ?? '')
      const placeholder = placeholders.get(image)
      const picture = (placeholder ?? image).closest('picture')
      // A file that is not there, or not read yet, shows its alt text, as an
      // image that does not load does; so does the image of a picture that
      // cannot choose its source yet.
      if (shown === undefined || shown.state !== 'shown' || (picture && choosing.has(picture))) {
        if (!placeholder)
          fail(image)
        continue
      }
      if (placeholder) {
        // An image whose bytes did not load tries again only for a version
        // it has not tried; one that waited shows once it can.
        if (unloaded.has(placeholder) && shown.src === image.getAttribute('src'))
          continue
        placeholder.replaceWith(image)
        failed.delete(placeholder)
        unloaded.delete(placeholder)
        // Its own set, which the pass above did not find while it was away.
        if (image.hasAttribute(HOST_SRCSET_ATTRIBUTE))
          setSrcset(image)
      }
      image.setAttribute('src', shown.src)
    }
  }

  /**
   * Writes the `srcset` of `element` from its candidates' files, once each
   * is read; whether it could, or a file is still being read.
   */
  function setSrcset(element: Element): boolean {
    const candidates = []
    for (const candidate of hostSrcsetCandidates(element)) {
      if ('url' in candidate) {
        candidates.push(candidate)
        continue
      }
      const shown = hostImages.value.get(candidate.path)
      // Until every file is read, the set could pick a candidate it would not pick whole.
      if (shown === undefined || shown.state === 'loading') {
        element.removeAttribute('srcset')
        return false
      }
      if (shown.state === 'shown')
        candidates.push({ url: shown.src, descriptor: candidate.descriptor })
    }
    const srcset = srcsetText(candidates)
    if (srcset === '')
      element.removeAttribute('srcset')
    // Set only when it changes, since setting it makes the web browser choose again.
    else if (element.getAttribute('srcset') !== srcset)
      element.setAttribute('srcset', srcset)
    return true
  }
  watch([html, hostImages], address, { immediate: true, flush: 'post' })

  return { onError, reload }
}
