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
 * of its file's version; a candidate whose file is not there drops out.
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
  const failed = new Map<HTMLElement, HTMLImageElement>()

  /** Shows `image`'s alt text in its place. */
  function fail(image: HTMLImageElement): void {
    const made = document.createElement('template')
    made.innerHTML = imagePlaceholderHtml(image.alt)
    const placeholder = made.content.firstElementChild
    if (!(placeholder instanceof HTMLElement))
      return
    failed.set(placeholder, image)
    image.replaceWith(placeholder)
  }

  function onError(event: Event): void {
    if (event.target instanceof HTMLImageElement)
      fail(event.target)
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
    }
  }

  // A new rendering brings its own images.
  watch(html, () => failed.clear())

  async function address(): Promise<void> {
    // The rendering reaches the page first.
    await nextTick()
    const placeholders = new Map([...failed].map(([placeholder, image]) => [image, placeholder]))
    const images = [...options.root()?.querySelectorAll<HTMLImageElement>(`img[${HOST_FILE_ATTRIBUTE}]`) ?? [], ...placeholders.keys()]
    for (const image of images) {
      const shown = hostImages.value.get(image.getAttribute(HOST_FILE_ATTRIBUTE) ?? '')
      const placeholder = placeholders.get(image)
      // A file that is not there, or not read yet, shows its alt text, as an image that does not load does.
      if (shown === undefined || shown.state !== 'shown') {
        if (!placeholder)
          fail(image)
        continue
      }
      if (placeholder) {
        // A failed image tries again only for a version it has not tried.
        if (shown.src === image.getAttribute('src'))
          continue
        placeholder.replaceWith(image)
        failed.delete(placeholder)
      }
      image.setAttribute('src', shown.src)
    }
    for (const element of options.root()?.querySelectorAll(`[${HOST_SRCSET_ATTRIBUTE}]`) ?? [])
      setSrcset(element)
  }

  /** Writes the `srcset` of `element` from its candidates' files, once each is read. */
  function setSrcset(element: Element): void {
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
        return
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
  }
  watch([html, hostImages], address, { immediate: true, flush: 'post' })

  return { onError, reload }
}
