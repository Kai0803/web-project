<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { api, formatMoney, type Project, type Quotation } from '@/api/client'
import { useAuth } from '@/stores/auth'
import ErrorAlert from '@/components/ErrorAlert.vue'
import StatusTag from '@/components/StatusTag.vue'

const props = defineProps<{ id: string }>()
const auth = useAuth()
const router = useRouter()

const project = ref<Project | null>(null)
const quotations = ref<Quotation[]>([])
const error = ref<unknown>(null)
const loading = ref(true)
const creating = ref(false)

async function load() {
  loading.value = true
  try {
    const id = Number(props.id)
    project.value = await api.getProject(id)
    quotations.value = (await api.listQuotations(id)).items
  } catch (err) {
    error.value = err
  } finally {
    loading.value = false
  }
}

/** 建立一張空白報價單，直接進編輯畫面加明細。 */
async function createQuotation() {
  creating.value = true
  error.value = null
  try {
    const created = await api.createQuotation(Number(props.id), [])
    await router.push(`/quotations/${created.id}`)
  } catch (err) {
    error.value = err
  } finally {
    creating.value = false
  }
}

onMounted(load)
</script>

<template>
  <ErrorAlert :error="error" />

  <div v-if="project">
    <div class="page-head">
      <div>
        <h1>{{ project.name }}</h1>
        <p>
          <span class="code">{{ project.code }}</span>
          <template v-if="project.site_address">　·　{{ project.site_address }}</template>
        </p>
      </div>
      <div class="btn-row">
        <StatusTag :status="project.status" />
        <button v-if="auth.can('quotation.write')" class="btn" :disabled="creating" @click="createQuotation">
          {{ creating ? '建立中…' : '新增報價單' }}
        </button>
      </div>
    </div>

    <div class="card">
      <div class="card-head"><h2>報價單</h2></div>
      <table>
        <thead>
          <tr>
            <th>單號</th><th>版本</th><th>狀態</th>
            <th class="num">未稅</th><th class="num">稅額</th><th class="num">含稅總計</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="q in quotations" :key="q.id">
            <td class="code"><RouterLink :to="`/quotations/${q.id}`">{{ q.quote_no }}</RouterLink></td>
            <td class="num">v{{ q.version }}</td>
            <td><StatusTag :status="q.status" /></td>
            <td class="num">{{ formatMoney(q.subtotal) }}</td>
            <td class="num">{{ formatMoney(q.tax_amount) }}</td>
            <td class="num"><strong>{{ formatMoney(q.total) }}</strong></td>
          </tr>
        </tbody>
      </table>
      <div v-if="loading" class="empty">載入中…</div>
      <div v-else-if="!quotations.length" class="empty">尚無報價單。</div>
    </div>
  </div>
</template>
