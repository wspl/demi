// KaTeX and its stylesheet, which names its fonts: a chunk of its own, which
// `math.ts` loads, with the fonts, once the first conversation has shown or
// the first math shown asks, so the first conversation downloads neither
// first.
import 'katex/dist/katex.min.css'

export { default } from 'katex'
