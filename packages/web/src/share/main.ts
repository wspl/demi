import SharedLinkMissing from '@demicodes/web-ui/share/SharedLinkMissing.vue'
import { mountLinkPage } from '../accountless/mount'
import SharePage from './SharePage.vue'
import '../style.css'

/*
 * The shared page's composition root: its own small entry of the build, which
 * loads none of the product's and calls only the share's public routes.
 */
mountLinkPage('s', SharedLinkMissing, SharePage)
