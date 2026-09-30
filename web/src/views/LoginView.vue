<script setup lang="ts">
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useAuth } from '@/stores/auth'
import BrandMark from '@/components/BrandMark.vue'
import ErrorAlert from '@/components/ErrorAlert.vue'

const auth = useAuth()
const route = useRoute()
const router = useRouter()

const username = ref('')
const password = ref('')
const error = ref<unknown>(null)
const busy = ref(false)

async function submit() {
  error.value = null
  busy.value = true
  try {
    await auth.login(username.value, password.value)
    const next = typeof route.query.next === 'string' ? route.query.next : '/projects'
    await router.push(next)
  } catch (err) {
    error.value = err
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="login-wrap">
    <form class="login-card" @submit.prevent="submit">
      <div class="brand"><BrandMark /><span>空調配管工程管理系統</span></div>

      <ErrorAlert :error="error" />

      <div class="field">
        <label for="u">帳號</label>
        <input id="u" v-model="username" autocomplete="username" required autofocus />
      </div>
      <div class="field">
        <label for="p">密碼</label>
        <input id="p" v-model="password" type="password" autocomplete="current-password" required />
      </div>

      <button class="btn" style="width: 100%" :disabled="busy">{{ busy ? '登入中…' : '登入' }}</button>
    </form>
  </div>
</template>
