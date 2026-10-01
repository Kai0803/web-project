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
  /**
   * 這一列載入時的伺服器 id，新增的列為 null。
   *
   * 儲存時原樣送回，伺服器據此承接我們無權限檢視的欄位（成本）。
   * 少了它，伺服器只能依陣列位置對應，刪掉中間一列就會把成本接到錯的列上。
   */
  id: number | null
  /** 本地的穩定鍵，供 v-for 使用。新增的列還沒有伺服器 id。 */
  uid: number
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

/** v-for 的穩定鍵來源。只在本頁有意義，不送到伺服器。 */
let nextUid = 1

function toDraft(d: QuotationDetail): DraftLine[] {
  return d.items.map((i) => ({
    id: i.id,
    uid: nextUid++,
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
    id: null,
    uid: nextUid++,
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
      // 原樣送回，讓伺服器認得出這是哪一列
      id: l.id,
      name: l.name.trim(),
      spec: l.spec.trim() || null,
      unit: l.unit.trim(),
      // 後端以字串解析 Decimal，前後空白會讓整個請求被拒。
      // 數值本身不在前端動，只去掉空白。
      quantity: l.quantity.trim(),
      unit_price: l.unit_price.trim(),
      unit_cost: l.unit_cost.trim() || null,
      note: l.note.trim() || null,
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
 * 稅率顯示，例如 `5%`。
 *
 * 由伺服器算好後直接顯示。前端原本有一份自己的字串移位實作，
 * 在小數位數為 1 時算錯（`0.1` 顯示成 `1%`）；同一個規則有兩份實作
 * 就會有一份是錯的，而稅率是會印在報價單上的數字。
 */
const taxPercent = computed(() => detail.value?.tax_percent ?? '0%')

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
        <a class="btn ghost" :href="`/api/v1/quotations/${detail.id}/pdf`" target="_blank" rel="noopener">
          下載 PDF
        </a>
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
          <!-- 鍵必須穩定：用索引的話，刪掉中間一列會讓 Vue 以位置重用
               DOM，正在輸入的 IME 組字與焦點會跳到別列上 -->
          <tr v-for="(line, index) in lines" :key="line.uid">
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
