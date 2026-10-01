<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { RouterLink } from 'vue-router'
import { api, ApiError, type ProjectListItem, type CustomerOption } from '@/api/client'
import { useAuth } from '@/stores/auth'
import ErrorAlert from '@/components/ErrorAlert.vue'
import StatusTag from '@/components/StatusTag.vue'
import Pager from '@/components/Pager.vue'

const auth = useAuth()
const projects = ref<ProjectListItem[]>([])
const customers = ref<CustomerOption[]>([])
const page = ref({ total: 0, limit: 50, offset: 0 })
const error = ref<unknown>(null)
const loading = ref(true)

const creating = ref(false)
const form = ref({ customer_id: 0, name: '', site_address: '' })
const formError = ref<ApiError | null>(null)
const busy = ref(false)

async function load() {
  loading.value = true
  try {
    const result = await api.listProjects({ limit: page.value.limit, offset: page.value.offset })
    projects.value = result.items
    page.value = { total: result.total, limit: result.limit, offset: result.offset }
    // 選單用不分頁的 options 端點。用分頁列表填選單的話，
    // 客戶數超過一頁之後就會有客戶無法被選為新專案的客戶。
    if (auth.can('customer.read')) customers.value = await api.customerOptions()
  } catch (err) {
    error.value = err
  } finally {
    loading.value = false
  }
}

function goTo(offset: number) {
  page.value.offset = offset
  void load()
}

async function create() {
  formError.value = null
  busy.value = true
  try {
    await api.createProject({
      customer_id: form.value.customer_id,
      name: form.value.name,
      site_address: form.value.site_address || null,
      note: null,
    })
    form.value = { customer_id: 0, name: '', site_address: '' }
    creating.value = false
    await load()
  } catch (err) {
    formError.value = err instanceof ApiError ? err : null
    if (!formError.value) error.value = err
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="page-head">
    <div>
      <h1>專案</h1>
      <p>每個案場一個專案，報價與合約都掛在它底下。</p>
    </div>
    <button v-if="auth.can('project.write')" class="btn" @click="creating = !creating">
      {{ creating ? '取消' : '新增專案' }}
    </button>
  </div>

  <ErrorAlert :error="error" />

  <div v-if="creating" class="card">
    <div class="card-head"><h2>新增專案</h2></div>
    <form class="card-body" @submit.prevent="create">
      <ErrorAlert :error="formError && !formError.fields ? formError : null" />
      <div class="row">
        <div class="field">
          <label for="c">客戶</label>
          <select id="c" v-model.number="form.customer_id" required>
            <option :value="0" disabled>請選擇客戶</option>
            <option v-for="c in customers" :key="c.id" :value="c.id">{{ c.code }}　{{ c.name }}</option>
          </select>
          <span v-if="!customers.length" class="hint">尚無客戶，請先於「客戶」頁面建立。</span>
        </div>
        <div class="field">
          <label for="pn">案場名稱</label>
          <input id="pn" v-model="form.name" required :aria-invalid="!!formError?.fieldError('name')" />
          <span v-if="formError?.fieldError('name')" class="error">{{ formError.fieldError('name') }}</span>
        </div>
      </div>
      <div class="field"><label for="sa">施工地址</label><input id="sa" v-model="form.site_address" /></div>
      <button class="btn" :disabled="busy || !form.customer_id">{{ busy ? '建立中…' : '建立' }}</button>
    </form>
  </div>

  <div class="card">
    <table>
      <thead>
        <tr><th>案號</th><th>案場</th><th>客戶</th><th>狀態</th></tr>
      </thead>
      <tbody>
        <tr v-for="p in projects" :key="p.id">
          <td class="code"><RouterLink :to="`/projects/${p.id}`">{{ p.code }}</RouterLink></td>
          <td>{{ p.name }}</td>
          <td class="muted">{{ p.customer_name }}</td>
          <td><StatusTag :status="p.status" /></td>
        </tr>
      </tbody>
    </table>
    <Pager v-bind="page" :busy="loading" @go="goTo" />
    <div v-if="loading" class="empty">載入中…</div>
    <div v-else-if="!projects.length" class="empty">尚無專案。</div>
  </div>
</template>
