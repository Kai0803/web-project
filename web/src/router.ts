import { createRouter, createWebHistory } from 'vue-router'
import { useAuth } from '@/stores/auth'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', name: 'login', component: () => import('@/views/LoginView.vue'), meta: { public: true } },
    { path: '/', redirect: '/projects' },
    { path: '/projects', name: 'projects', component: () => import('@/views/ProjectsView.vue') },
    { path: '/projects/:id', name: 'project', component: () => import('@/views/ProjectView.vue'), props: true },
    { path: '/quotations/:id', name: 'quotation', component: () => import('@/views/QuotationView.vue'), props: true },
    { path: '/customers', name: 'customers', component: () => import('@/views/CustomersView.vue') },
    { path: '/settings', name: 'settings', component: () => import('@/views/SettingsView.vue') },
    { path: '/users', name: 'users', component: () => import('@/views/UsersView.vue') },
    { path: '/:pathMatch(.*)*', component: () => import('@/views/NotFoundView.vue') },
  ],
})

/**
 * 未登入者一律導向登入頁。
 *
 * 這只是體驗上的引導——真正的管制在後端，每個端點都自己檢查。
 * 繞過這個 guard 也拿不到任何資料。
 */
router.beforeEach(async (to) => {
  const auth = useAuth()
  if (!auth.ready) await auth.refresh()

  if (to.meta.public) {
    return auth.isAuthenticated && to.name === 'login' ? { name: 'projects' } : true
  }
  return auth.isAuthenticated ? true : { name: 'login', query: { next: to.fullPath } }
})
