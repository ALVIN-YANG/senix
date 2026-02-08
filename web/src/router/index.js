import { createRouter, createWebHistory } from 'vue-router'
import { useUserStore } from '@/stores/user'

const routes = [
  {
    path: '/login',
    name: 'Login',
    component: () => import('@/views/Login.vue'),
    meta: { public: true }
  },
  {
    path: '/',
    name: 'Layout',
    component: () => import('@/layouts/MainLayout.vue'),
    redirect: '/dashboard',
    children: [
      {
        path: 'dashboard',
        name: 'Dashboard',
        component: () => import('@/views/Dashboard.vue'),
        meta: { title: '概览', icon: 'HomeFilled' }
      },
      {
        path: 'sites',
        name: 'Sites',
        component: () => import('@/views/Sites.vue'),
        meta: { title: '站点管理', icon: 'OfficeBuilding' }
      },
      {
        path: 'certs',
        name: 'Certificates',
        component: () => import('@/views/Certificates.vue'),
        meta: { title: '证书管理', icon: 'Lock' }
      },
      {
        path: 'security',
        name: 'Security',
        component: () => import('@/views/Security.vue'),
        meta: { title: '安全策略', icon: 'Shield' }
      },
      {
        path: 'settings',
        name: 'Settings',
        component: () => import('@/views/Settings.vue'),
        meta: { title: '系统设置', icon: 'Setting' }
      }
    ]
  }
]

const router = createRouter({
  history: createWebHistory(),
  routes
})

// 路由守卫
router.beforeEach((to, from, next) => {
  const userStore = useUserStore()
  
  if (to.meta.public) {
    next()
    return
  }
  
  if (!userStore.token) {
    next('/login')
    return
  }
  
  next()
})

export default router
