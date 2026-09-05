import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import { useConversations } from './conversation/store'
import { useResources } from './prototype/resources'
import ChatPage from './conversation/ChatPage.vue'
import LoginPage from './auth/LoginPage.vue'
import App from './App.vue'
import './style.css'

const pinia = createPinia()
const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/chat/welcome' },
    { path: '/chat/:id?', component: ChatPage },
    { path: '/login', component: LoginPage },
    { path: '/:pathMatch(.*)*', redirect: '/chat' },
  ],
})
const resources = useResources(pinia)
const conversations = useConversations(pinia)
router.beforeEach((to) => (!resources.signedIn && to.path !== '/login' ? '/login' : true))
applyThemeToDocument()
createApp(App).use(pinia).use(router).mount('#app')
const timer = window.setInterval(() => conversations.advance(), 80)
if (import.meta.hot) import.meta.hot.dispose(() => clearInterval(timer))
