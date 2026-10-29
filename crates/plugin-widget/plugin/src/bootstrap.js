// A widget's bootstrap script, a function of the default look's root
// attributes (`page.rs`): the reply's appearance on the page's root, as the
// frame's name gives it, and the page's height for a frame that follows it.
(defaultLook) => {
  const root = document.documentElement
  const apply = (look) => {
    for (const [name, value] of Object.entries(look.attributes ?? {})) root.setAttribute(name, value)
    for (const [name, value] of Object.entries(look.properties ?? {})) root.style.setProperty(name, value)
  }
  try {
    apply(JSON.parse(window.name))
  } catch {
    // A page named nothing, as one opened in a tab of its own, draws in the
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
}
