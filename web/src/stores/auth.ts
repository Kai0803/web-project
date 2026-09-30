/**
 * 目前登入者。
 *
 * 這是唯一放進全域 store 的狀態——它真的跨頁共用。
 * 客戶、專案、報價單等資料留在各自的畫面，不上提。
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { api, type Me } from '@/api/client'

export const useAuth = defineStore('auth', () => {
  const me = ref<Me | null>(null)
  const ready = ref(false)

  /**
   * 是否具備某權限。
   *
   * 僅用於決定「要不要顯示這個按鈕」。實際管制在後端——
   * 前端隱藏只是體驗，不是安全機制。
   */
  function can(permission: string): boolean {
    return me.value?.permissions.includes(permission) ?? false
  }

  async function refresh() {
    try {
      me.value = await api.me()
    } catch {
      me.value = null
    } finally {
      ready.value = true
    }
  }

  async function login(username: string, password: string) {
    await api.login(username, password)
    me.value = await api.me()
  }

  async function logout() {
    await api.logout()
    me.value = null
  }

  return {
    me,
    ready,
    isAuthenticated: computed(() => me.value !== null),
    can,
    refresh,
    login,
    logout,
  }
})
