// The demi.sh site: one page that shows what Demi is and how to install it.
//
// `build.ts` renders the page into the HTML at build time; this entry then
// takes that markup over in the browser (hydration). Under `vite` (the
// development server) there is no markup yet, and the page is mounted fresh.
//
// The page imports nothing from the product: the feature tour's demos are
// the product's own app frame, in frames of their own (`demo/main.ts`), which
// the page loads only as the tour nears the viewport.

import { createApp, createSSRApp } from 'vue'
import App from './App.vue'
import './style.css'

const root = document.querySelector('#app')
if (!root)
  throw new Error('index.html has no #app')
const app = root.firstElementChild ? createSSRApp(App) : createApp(App)
app.mount(root)
