import ViewLinkGone from '@demicodes/web-ui/view/ViewLinkGone.vue'
import { mountLinkPage } from '../accountless/mount'
import ViewPage from './ViewPage.vue'
import '../style.css'

/*
 * A view link's page's composition root: its own small entry of the build,
 * as a share's is, which loads none of the product's and reaches only the
 * link's public routes.
 */
mountLinkPage('v', ViewLinkGone, ViewPage)
