<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { api, formatMoney, ApiError, type QuotationDetail, type LineRequest } from '@/api/client'
import { useAuth } from '@/stores/auth'
import ErrorAlert from '@/components/ErrorAlert.vue'
import StatusTag from '@/components/StatusTag.vue'

const props = defineProps<{ id: string }>()
const auth = useAuth()
const router = useRouter()

const detail = ref<QuotationDetail | null>(null)
const error = ref<unknown>(null)
const notice = ref<string | null>(null)
const loading = ref(true)
const busy = ref(false)

/** 編輯中的明細。與伺服器回傳的 items 分開，儲存後才同步。 */
type DraftLine = {
  name: string
  spec: string
  unit: string
  quantity: string
  unit_price: string
  unit_cost: string
  note: string
  /** 上次儲存時伺服器算出的小計。編輯後即失效。 */
  savedAmount: string | null
}

const lines = ref<DraftLine[]>([])
const dirty = ref(false)

const editable = computed(() => detail.value?.status === 'draft' && auth.can('quotation.write'))
const canSeeCost = computed(() => auth.can('quotation.cost.read'))

function toDraft(d: QuotationDetail): DraftLine[] {
  return d.items.map((i) => ({
    name: i.name,
    spec: i.spec ?? '',
    unit: i.unit,
    quantity: i.quantity,
    unit_price: i.unit_price,
    unit_cost: i.unit_cost ?? '',
    note: i.note ?? '',
    savedAmount: i.amount,
  }))
}

async function load() {
  loading.value = true
  try {
    const d = await api.getQuotation(Number(props.id))
    detail.value = d
    lines.value = toDraft(d)
    dirty.value = false
  } catch (err) {
    error.value = err
  } finally {
    loading.value = false
  }
}

function addLine() {
  lines.value.push({
    name: '',
    spec: '',
    unit: '式',
    quantity: '1',
    unit_price: '0',
    unit_cost: '',
    note: '',
    savedAmount: null,
  })
  dirty.value = true
}

function removeLine(index: number) {
  lines.value.splice(index, 1)
  dirty.value = true
}

/** 任何欄位變動都讓小計失效，直到伺服器重新算過。 */
function touch(line: DraftLine) {
  line.savedAmount = null
  dirty.value = true
}

/**
 * 儲存明細。
 *
 * 金額由伺服器依進位規則計算後回傳——前端不做試算。
 * 這保證畫面上的數字與列印出來的報價單完全一致，
 * 也讓進位邏輯只有一份實作。
 */
async function save() {
  busy.value = true
  error.value = null
  notice.value = null
  try {
    const payload: LineRequest[] = lines.value.map((l) => ({
      name: l.name,
      spec: l.spec || null,
      unit: l.unit,
      quantity: l.quantity,
      unit_price: l.unit_price,
      unit_cost: l.unit_cost || null,
      note: l.note || null,
    }))
    const updated = await api.replaceItems(Number(props.id), payload)
    detail.value = updated
    lines.value = toDraft(updated)
    dirty.value = false
    notice.value = '已儲存，金額由伺服器重新計算。'
  } catch (err) {
    error.value = err
  } finally {
    busy.value = false
  }
}

async function act(fn: () => Promise<unknown>, message: string) {
  busy.value = true
  error.value = null
  notice.value = null
  try {
    await fn()
    await load()
    notice.value = message
  } catch (err) {
    error.value = err
  } finally {
    busy.value = false
  }
}

const submit = () => act(() => api.submit(Number(props.id)), '已送審。')
const approve = () => act(() => api.approve(Number(props.id)), '已核准。')
const reject = () => act(() => api.reject(Number(props.id)), '已駁回。')
const reopen = () => act(() => api.reopen(Number(props.id)), '已退回草稿。')

async function newVersion() {
  busy.value = true
  error.value = null
  try {
    const created = await api.newVersion(Number(props.id))
    await router.push(`/quotations/${created.id}`)
    await load()
  } catch (err) {
    error.value = err
  } finally {
    busy.value = false
  }
}

async function toContract() {
  busy.value = true
  error.value = null
  try {
    const today = new Date().toISOString().slice(0, 10)
    const contract = await api.toContract(Number(props.id), today)
    notice.value = `已轉為合約 ${contract.contract_no}，報價明細已建立為工項。`
    await load()
  } catch (err) {
    error.value = err
  } finally {
    busy.value = false
  }
}

/**
 * 稅率顯示，例如 `0.0500` → `5%`。
 *
 * 純字串運算。稅率本身不是金額，但整條鏈既然刻意避開浮點，
 * 這裡也不開例外，免得日後被當成「可以用 Number()」的先例。
 */
const taxPercent = computed(() => {
  const rate = detail.value?.tax_rate ?? '0'
  const [whole = '0', fraction = ''] = rate.split('.')
  const digits = (whole + fraction).replace(/^0+/, '') || '0'
  const shifted = fraction.length >= 2 ? fraction.length - 2 : 0
  const scaled = digits.padStart(shifted + 1, '0')
  const cut = scaled.length - shifted
  const result = `${scaled.slice(0, cut)}${shifted ? '.' + scaled.slice(cut) : ''}`
  return `${result.replace(/\.?0+$/, '')}%`
})

/** 樂觀鎖衝突時給出具體的下一步，而不是丟一句「衝突」。 */
const conflictHint = computed(
  () => error.value instanceof ApiError && error.value.code === 'CONFLICT',
)

onMounted(load)
</script>

<template>
  <ErrorAlert :error="error" />
  <div v-if="conflictHint" class="alert info">這份報價單已被他人修改，請重新整理頁面後再操作。</div>
  <div v-if="notice" class="alert ok">{{ notice }}</div>

  <div v-if="detail">
    <div class="page-head">
      <div>
        <h1>{{ detail.quote_no }}　<span class="faint">第 {{ detail.version }} 版</span></h1>
        <p>
          <RouterLink :to="`/projects/${detail.project_id}`">← 回到專案</RouterLink>
        </p>
      </div>
      <div class="btn-row">
        <StatusTag :status="detail.status" />

        <button v-if="detail.status === 'draft' && auth.can('quotation.write')"
                class="btn ghost" :disabled="busy || dirty" @click="submit">送審</button>
        <button v-if="detail.status === 'pending' && auth.can('quotation.approve')"
                class="btn" :disabled="busy" @click="approve">核准</button>
        <button v-if="detail.status === 'pending' && auth.can('quotation.approve')"
                class="btn danger" :disabled="busy" @click="reject">駁回</button>
        <button v-if="detail.status === 'rejected' && auth.can('quotation.write')"
                class="btn ghost" :disabled="busy" @click="reopen">退回草稿</button>
        <button v-if="detail.status !== 'superseded' && auth.can('quotation.write')"
                class="btn ghost" :disabled="busy" @click="newVersion">建立新版本</button>
        <button v-if="detail.status === 'approved' && auth.can('contract.write')"
                class="btn copper" :disabled="busy" @click="toContract">轉為合約</button>
      </div>
    </div>

    <div v-if="dirty" class="alert info">
      明細已變更但尚未儲存。<strong>金額由伺服器計算</strong>，請按「儲存明細」後才會更新。
    </div>
    <div v-if="detail.status !== 'draft'" class="alert info">
      此版本狀態為「{{ detail.status === 'approved' ? '已核准' : detail.status === 'pending' ? '送審中' : '唯讀' }}」，
      明細已鎖定。若需修改請建立新版本，舊版仍會保留供查閱。
    </div>

    <div class="card">
      <div class="card-head">
        <h2>明細</h2>
        <div v-if="editable" class="btn-row">
          <button class="btn ghost sm" :disabled="busy" @click="addLine">新增一列</button>
          <button class="btn sm" :disabled="busy || !dirty" @click="save">
            {{ busy ? '儲存中…' : '儲存明細' }}
          </button>
        </div>
      </div>

      <table>
        <thead>
          <tr>
            <th style="width: 26%">項目</th>
            <th style="width: 14%">規格</th>
            <th style="width: 7%">單位</th>
            <th class="num" style="width: 11%">數量</th>
            <th class="num" style="width: 13%">單價</th>
            <th v-if="canSeeCost" class="num" style="width: 12%">成本</th>
            <th class="num" style="width: 13%">小計</th>
            <th v-if="editable" style="width: 4%"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(line, index) in lines" :key="index">
            <template v-if="editable">
              <td><input v-model="line.name" @input="touch(line)" /></td>
              <td><input v-model="line.spec" @input="touch(line)" /></td>
              <td><input v-model="line.unit" @input="touch(line)" /></td>
              <td><input v-model="line.quantity" class="num" inputmode="decimal" @input="touch(line)" /></td>
              <td><input v-model="line.unit_price" class="num" inputmode="decimal" @input="touch(line)" /></td>
              <td v-if="canSeeCost"><input v-model="line.unit_cost" class="num" inputmode="decimal" @input="touch(line)" /></td>
              <td class="num">
                <span v-if="line.savedAmount">{{ formatMoney(line.savedAmount) }}</span>
                <span v-else class="faint" title="儲存後由伺服器計算">待計算</span>
              </td>
              <td><button class="btn danger sm" @click="removeLine(index)">刪</button></td>
            </template>
            <template v-else>
              <td>{{ line.name }}</td>
              <td class="muted">{{ line.spec || '—' }}</td>
              <td>{{ line.unit }}</td>
              <td class="num">{{ line.quantity }}</td>
              <td class="num">{{ formatMoney(line.unit_price) }}</td>
              <td v-if="canSeeCost" class="num muted">{{ line.unit_cost ? formatMoney(line.unit_cost) : '—' }}</td>
              <td class="num"><strong>{{ line.savedAmount ? formatMoney(line.savedAmount) : '—' }}</strong></td>
            </template>
          </tr>
        </tbody>
      </table>

      <div v-if="!lines.length" class="empty">
        尚無明細。{{ editable ? '請按「新增一列」開始。' : '' }}
      </div>

      <div class="card-body">
        <dl class="totals">
          <dt>未稅合計</dt>
          <dd>{{ formatMoney(detail.subtotal) }}</dd>
          <dt>營業稅（{{ taxPercent }}）</dt>
          <dd>{{ formatMoney(detail.tax_amount) }}</dd>
          <dt class="grand-label">含稅總計</dt>
          <dd class="grand">{{ formatMoney(detail.total) }}</dd>
        </dl>
        <p class="faint" style="text-align: right; margin: 10px 0 0">
          每列小計四捨五入至整數元，合計為各列相加——客戶逐列加總可對上此金額。
        </p>
      </div>
    </div>
  </div>

  <div v-else-if="loading" class="empty">載入中…</div>
</template>
