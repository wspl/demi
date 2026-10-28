// The share image's page: `build.ts` opens it in a browser and photographs
// its card into `og.png`, the picture a link to the site shows when shared.
import { createApp } from 'vue'
import ShareCard from './ShareCard.vue'
import '../style.css'

createApp(ShareCard).mount('#app')
