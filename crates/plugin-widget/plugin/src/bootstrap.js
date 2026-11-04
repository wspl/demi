// A widget's bootstrap script, a function of the default look's root
// attributes and the address of html-to-image (`page.rs`): the reply's
// appearance on the page's root, as the frame's name gives it, the page's
// height for a frame that follows it, and a picture of the page as it shows
// now, which the frame's parent asks for to copy it.
(defaultLook, pictureLibrary) => {
  const root = document.documentElement
  const apply = (look) => {
    for (const [name, value] of Object.entries(look.attributes ?? {})) root.setAttribute(name, value)
    for (const [name, value] of Object.entries(look.properties ?? {})) root.style.setProperty(name, value)
  }
  try {
    apply(JSON.parse(window.name))
  } catch {
    // A page named nothing, as its screenshots' page, draws in the
    // default look and the system's light or dark, on the surface a reply
    // would give it.
    const theme = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
    apply({ attributes: { ...defaultLook, 'data-theme': theme } })
    root.style.background = 'var(--surface)'
  }
  // The lowest bottom of the body and of every element in it, which holds
  // what is positioned out of the flow; what an element that clips holds,
  // a scroller's rows, is its own and counts no further than it does.
  const clips = (element) => getComputedStyle(element).overflowY !== 'visible'
  const height = () => {
    let bottom = document.body.getBoundingClientRect().bottom
    const walk = (element) => {
      for (const child of element.children) {
        bottom = Math.max(bottom, child.getBoundingClientRect().bottom)
        if (!clips(child)) walk(child)
      }
    }
    // The body's overflow is the viewport's, which clips nothing it holds.
    walk(document.body)
    return Math.ceil(bottom + scrollY)
  }
  const report = () => parent.postMessage({ widget: 'height', height: height() }, '*')
  // Reported as the page loads and as its body changes size; a body of no
  // height, whose content is all positioned, gets no first observation.
  addEventListener('DOMContentLoaded', () => {
    report()
    new ResizeObserver(report).observe(document.body)
  })
  addEventListener('load', report)

  // The picture: the frame's box as the page draws it now, at twice its CSS
  // pixels, on the surface the page draws on. The frame's parent cannot draw
  // it, since it cannot read an opaque origin. The library loads at the first
  // ask, and again at the next after one that failed. It runs with the page's
  // own `define`, `exports` and `module` out of its sight, so an AMD loader
  // or a CommonJS shim of the page does not take its exports from `window`.
  let library = null
  const load = async () => {
    const answer = await fetch(pictureLibrary).catch(() => undefined)
    if (!answer?.ok)
      throw new Error(`the image library did not load${answer ? `: ${answer.status}` : ''}`)
    new Function('define', 'exports', 'module', await answer.text()).call(window)
    if (!window.htmlToImage)
      throw new Error('the image library did not load')
    return window.htmlToImage
  }
  const surface = () => {
    const probe = document.createElement('div')
    probe.style.background = 'var(--color-surface, var(--surface))'
    document.body.append(probe)
    const color = getComputedStyle(probe).backgroundColor
    probe.remove()
    return color
  }
  addEventListener('message', async (event) => {
    if (event.source !== parent || event.data?.widget !== 'picture')
      return
    const { request } = event.data
    try {
      library ??= load().catch((error) => {
        library = null
        throw error
      })
      const { toBlob } = await library
      const picture = await toBlob(root, {
        pixelRatio: 2,
        width: root.clientWidth,
        height: root.clientHeight,
        backgroundColor: surface(),
      })
      if (!picture)
        throw new Error('the page drew no picture')
      parent.postMessage({ widget: 'picture', request, picture }, '*')
    } catch (error) {
      parent.postMessage({ widget: 'picture', request, error: String(error?.message ?? error) }, '*')
    }
  })
}
