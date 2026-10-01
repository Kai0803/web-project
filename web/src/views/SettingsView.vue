<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { api, type CompanyProfile } from '@/api/client'
import { useAuth } from '@/stores/auth'
import ErrorAlert from '@/components/ErrorAlert.vue'

const auth = useAuth()
const profile = ref<CompanyProfile | null>(null)
const error = ref<unknown>(null)
const notice = ref<string | null>(null)
const busy = ref(false)

const editable = computed(() => auth.can('settings.manage'))

/**
 * Logo 以 data URL 餵給 `<img>`，不用 `v-html`。
 *
 * `v-html` 走 innerHTML，貼進去的「SVG」只要含 `<img src=x onerror=...>`
 * 或 `<svg onload=...>` 就會在開啟此頁者的 origin 內執行——而公司資料
 * 任何登入者皆可讀取，等於持有 settings.manage 的人可以在 admin 的瀏覽器裡
 * 代為執行任意操作（三道 CSRF 防護對同源腳本全部無效）。
 * `<img src>` 載入的 SVG 不會執行腳本，也碰不到父文件。
 */
const logoDataUrl = computed(() => {
  const svg = profile.value?.logo_svg
  if (!svg) return null
  try {
    // btoa 只吃 Latin-1，SVG 可能含中文（例如 <title>）
    return `data:image/svg+xml;base64,${btoa(String.fromCharCode(...new TextEncoder().encode(svg)))}`
  } catch {
    return null
  }
})

async function load() {
  try {
    profile.value = await api.getCompany()
  } catch (err) {
    error.value = err
  }
}

async function save() {
  if (!profile.value) return
  busy.value = true
  error.value = null
  notice.value = null
  try {
    await api.updateCompany(profile.value)
    notice.value = '已儲存。下一份產生的報價單即採用新內容。'
  } catch (err) {
    error.value = err
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="page-head">
    <div>
      <h1>公司設定</h1>
      <p>這裡填的抬頭、Logo 與條款會直接印在報價單上。</p>
    </div>
  </div>

  <ErrorAlert :error="error" />
  <div v-if="notice" class="alert ok">{{ notice }}</div>
  <div v-if="!editable" class="alert info">你可以檢視這些資料，但需要「系統設定」權限才能修改。</div>

  <form v-if="profile" class="card" @submit.prevent="save">
    <div class="card-head"><h2>基本資料</h2></div>
    <div class="card-body">
      <div class="field">
        <label for="cn">公司抬頭</label>
        <input id="cn" v-model="profile.name" :disabled="!editable" placeholder="例如：〇〇空調工程有限公司" />
      </div>
      <div class="row">
        <div class="field"><label for="ct">統一編號</label><input id="ct" v-model="profile.tax_id" :disabled="!editable" /></div>
        <div class="field"><label for="cp">電話</label><input id="cp" v-model="profile.phone" :disabled="!editable" /></div>
        <div class="field"><label for="cf">傳真</label><input id="cf" v-model="profile.fax" :disabled="!editable" /></div>
      </div>
      <div class="row">
        <div class="field"><label for="ce">Email</label><input id="ce" v-model="profile.email" :disabled="!editable" /></div>
        <div class="field"><label for="ca">地址</label><input id="ca" v-model="profile.address" :disabled="!editable" /></div>
      </div>
    </div>

    <div class="card-head"><h2>Logo</h2></div>
    <div class="card-body">
      <div style="display: flex; gap: 20px; align-items: flex-start; flex-wrap: wrap">
        <img
          v-if="logoDataUrl"
          :src="logoDataUrl"
          alt="公司 Logo"
          style="width: 88px; height: 88px; border: 1px solid var(--line); border-radius: 8px; padding: 10px; background: #fff; object-fit: contain"
        />
        <div class="field" style="flex: 1; min-width: 260px">
          <label for="logo">SVG 原始碼</label>
          <textarea id="logo" v-model="profile.logo_svg" rows="5" :disabled="!editable" style="font-family: var(--mono); font-size: 0.82rem" />
          <span class="hint">網頁與報價單 PDF 共用同一份。換 Logo 只需貼上新的 SVG。</span>
        </div>
      </div>
    </div>

    <div class="card-head"><h2>報價單條款</h2></div>
    <div class="card-body">
      <div class="field">
        <textarea v-model="profile.quote_terms" rows="16" :disabled="!editable" style="font-size: 0.88rem" />
        <span class="hint">逐字印在報價單末頁。付款條件、保固範圍與責任限制建議請會計師或律師看過。</span>
      </div>
      <button v-if="editable" class="btn" :disabled="busy">{{ busy ? '儲存中…' : '儲存' }}</button>
    </div>
  </form>
</template>
