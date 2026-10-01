<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api, ApiError, type Customer } from '@/api/client'
import { useAuth } from '@/stores/auth'
import ErrorAlert from '@/components/ErrorAlert.vue'
import Pager from '@/components/Pager.vue'

const auth = useAuth()
const customers = ref<Customer[]>([])
const error = ref<unknown>(null)
const loading = ref(true)
const page = ref({ total: 0, limit: 50, offset: 0 })

const creating = ref(false)
const form = ref({ name: '', tax_id: '', phone: '', address: '' })
const formError = ref<ApiError | null>(null)
const busy = ref(false)

async function load() {
  loading.value = true
  try {
    const result = await api.listCustomers({ limit: page.value.limit, offset: page.value.offset })
    customers.value = result.items
    page.value = { total: result.total, limit: result.limit, offset: result.offset }
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
    await api.createCustomer({
      name: form.value.name,
      tax_id: form.value.tax_id || null,
      phone: form.value.phone || null,
      address: form.value.address || null,
      note: null,
    })
    form.value = { name: '', tax_id: '', phone: '', address: '' }
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
      <h1>客戶</h1>
      <p>客戶編號由系統依年度自動產生。</p>
    </div>
    <button v-if="auth.can('customer.write')" class="btn" @click="creating = !creating">
      {{ creating ? '取消' : '新增客戶' }}
    </button>
  </div>

  <ErrorAlert :error="error" />

  <div v-if="creating" class="card">
    <div class="card-head"><h2>新增客戶</h2></div>
    <form class="card-body" @submit.prevent="create">
      <ErrorAlert :error="formError && !formError.fields ? formError : null" />
      <div class="row">
        <div class="field">
          <label for="n">客戶名稱</label>
          <input id="n" v-model="form.name" required :aria-invalid="!!formError?.fieldError('name')" />
          <span v-if="formError?.fieldError('name')" class="error">{{ formError.fieldError('name') }}</span>
        </div>
        <div class="field"><label for="t">統一編號</label><input id="t" v-model="form.tax_id" /></div>
      </div>
      <div class="row">
        <div class="field"><label for="ph">電話</label><input id="ph" v-model="form.phone" /></div>
        <div class="field"><label for="ad">地址</label><input id="ad" v-model="form.address" /></div>
      </div>
      <button class="btn" :disabled="busy">{{ busy ? '建立中…' : '建立' }}</button>
    </form>
  </div>

  <div class="card">
    <table>
      <thead>
        <tr><th>編號</th><th>名稱</th><th>統一編號</th><th>電話</th></tr>
      </thead>
      <tbody>
        <tr v-for="c in customers" :key="c.id">
          <td class="code">{{ c.code }}</td>
          <td>{{ c.name }}</td>
          <td class="code">{{ c.tax_id ?? '—' }}</td>
          <td>{{ c.phone ?? '—' }}</td>
        </tr>
      </tbody>
    </table>
    <Pager v-bind="page" :busy="loading" @go="goTo" />
    <div v-if="loading" class="empty">載入中…</div>
    <div v-else-if="!customers.length" class="empty">尚無客戶資料。</div>
  </div>
</template>
