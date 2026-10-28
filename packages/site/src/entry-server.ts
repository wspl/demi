// What `build.ts` and the development server run outside the browser: the
// page's markup as a string, and what it tells search engines (`seo.ts`).
import { createSSRApp } from 'vue'
import { renderToString } from 'vue/server-renderer'
import App from './App.vue'

export { SHARE_IMAGE, headHtml, textFiles } from './seo'

export function render(): Promise<string> {
  return renderToString(createSSRApp(App))
}
