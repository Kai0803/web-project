<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter, RouterLink, RouterView } from 'vue-router'
import { useAuth } from '@/stores/auth'
import BrandMark from '@/components/BrandMark.vue'

const auth = useAuth()
const route = useRoute()
const router = useRouter()

const showShell = computed(() => auth.isAuthenticated && route.name !== 'login')

/** 選單項目依權限顯示。後端仍會各自檢查，這裡只是不擺沒用的按鈕。 */
const links = computed(() =>
  [
    { to: '/projects', label: '專案', show: auth.can('project.read') },
    { to: '/customers', label: '客戶', show: auth.can('customer.read') },
    { to: '/users', label: '人員', show: auth.can('user.manage') },
    { to: '/settings', label: '公司設定', show: true },
  ].filter((l) => l.show),
)

async function logout() {
  await auth.logout()
  await router.push({ name: 'login' })
}
</script>

<template>
  <div v-if="!auth.ready" class="empty">載入中…</div>

  <div v-else-if="showShell" class="shell">
    <nav class="sidebar">
      <RouterLink to="/projects" class="brand">
        <BrandMark on-dark />
        <span>ACMS</span>
      </RouterLink>

      <RouterLink
        v-for="link in links"
        :key="link.to"
        :to="link.to"
        class="nav-link"
        :class="{ active: route.path.startsWith(link.to) }"
      >
        {{ link.label }}
      </RouterLink>

      <div class="sidebar-foot">
        <div>{{ auth.me?.display_name }}</div>
        <button class="btn ghost sm" style="margin-top: 8px; color: #b9c9d4; border-color: #3c5a6e" @click="logout">
          登出
        </button>
      </div>
    </nav>

    <main class="main">
      <RouterView />
    </main>
  </div>

  <RouterView v-else />
</template>
