/**
 * Dresses a same-origin frame's document as the page: the page's style sheets
 * and the attributes of its `<html>` that the theme, the language and the
 * text size read, kept in step as they change, as a style the development
 * server replaces or a theme the user picks.
 *
 * The frame's own canvas stays transparent, so what it holds stands on what
 * the page draws under the frame.
 */
export function mirrorPage(frame: Document): () => void {
  const page = document
  const copyAttributes = (): void => {
    const from = page.documentElement
    const to = frame.documentElement
    for (const attribute of [...to.attributes]) {
      if (!from.hasAttribute(attribute.name))
        to.removeAttribute(attribute.name)
    }
    for (const attribute of from.attributes)
      to.setAttribute(attribute.name, attribute.value)
    to.style.setProperty('background', 'transparent', 'important')
  }
  const copySheets = (): void => {
    for (const old of frame.head.querySelectorAll('[data-mirrored]'))
      old.remove()
    for (const sheet of page.head.querySelectorAll('link[rel="stylesheet"], style')) {
      const copy = sheet.cloneNode(true) as HTMLElement
      copy.setAttribute('data-mirrored', '')
      frame.head.append(copy)
    }
  }
  copyAttributes()
  copySheets()
  const body = frame.body
  // The page's body stands where the visual viewport does, above a phone's keyboard; the frame's stays at its top.
  body.style.setProperty('position', 'static', 'important')
  body.style.setProperty('height', 'auto', 'important')
  body.style.setProperty('margin', '0')
  body.style.setProperty('background', 'transparent', 'important')
  body.style.setProperty('overflow', 'hidden')

  const attributes = new MutationObserver(copyAttributes)
  attributes.observe(page.documentElement, { attributes: true })
  let pending = 0
  const sheets = new MutationObserver(() => {
    if (pending === 0)
      pending = requestAnimationFrame(() => {
        pending = 0
        copySheets()
      })
  })
  sheets.observe(page.head, { childList: true, subtree: true, characterData: true })
  return () => {
    attributes.disconnect()
    sheets.disconnect()
    cancelAnimationFrame(pending)
  }
}
