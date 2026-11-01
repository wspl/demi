// Demi's otter, as the brand's lockup draws it, for every place the site
// shows it apart from the wordmark.
import lockup from '../../../docs/assets/brand/lockup-light.svg?raw'

/** The otter's drawing: the lockup's first nested drawing. */
export const OTTER_SVG = /<svg x="0"[\s\S]*?<\/svg>/.exec(lockup)?.[0] ?? ''
