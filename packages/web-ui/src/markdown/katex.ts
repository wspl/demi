// KaTeX and its stylesheet, which names its fonts: a chunk of its own that
// the first math shown loads (`math.ts`), so a page without math downloads
// neither KaTeX nor its fonts.
import 'katex/dist/katex.min.css'

export { default } from 'katex'
