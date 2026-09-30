<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api, ApiError, type UserSummary, type RoleSummary } from '@/api/client'
import ErrorAlert from '@/components/ErrorAlert.vue'

const users = ref<UserSummary[]>([])
const roles = ref<RoleSummary[]>([])
const error = ref<unknown>(null)
const notice = ref<string | null>(null)
const loading = ref(true)

const creating = ref(false)
const form = ref({ username: '', password: '', display_name: '', roles: [] as string[] })
const formError = ref<ApiError | null>(null)
const busy = ref(false)

async function load() {
  loading.value = true
  try {
    users.value = await api.listUsers()
    roles.value = await api.listRoles()
  } catch (err) {
    error.value = err
  } finally {
    loading.value = false
  }
}

async function create() {
  formError.value = null
  notice.value = null
  busy.value = true
  try {
    await api.createUser(form.value)
    form.value = { username: '', password: '', display_name: '', roles: [] }
    creating.value = false
    notice.value = '帳號已建立，對方可立即登入。'
    await load()
  } catch (err) {
    formError.value = err instanceof ApiError ? err : null
    if (!formError.value) error.value = err
  } finally {
    busy.value = false
  }
}

async function toggleActive(user: UserSummary) {
  error.value = null
  notice.value = null
  try {
    await api.updateUser(user.id, { is_active: !user.is_active })
    notice.value = user.is_active
      ? `已停用 ${user.display_name}，其登入狀態同時失效。`
      : `已重新啟用 ${user.display_name}。`
    await load()
  } catch (err) {
    error.value = err
  }
}

function roleName(code: string): string {
  return roles.value.find((r) => r.code === code)?.name ?? code
}

onMounted(load)
</script>

<template>
  <div class="page-head">
    <div>
      <h1>人員</h1>
      <p>一人可兼多個角色，權限取所有角色的聯集。</p>
    </div>
    <button class="btn" @click="creating = !creating">{{ creating ? '取消' : '新增帳號' }}</button>
  </div>

  <ErrorAlert :error="error" />
  <div v-if="notice" class="alert ok">{{ notice }}</div>

  <div v-if="creating" class="card">
    <div class="card-head"><h2>新增帳號</h2></div>
    <form class="card-body" @submit.prevent="create">
      <ErrorAlert :error="formError && !formError.fields ? formError : null" />
      <div class="row">
        <div class="field">
          <label for="un">登入帳號</label>
          <input id="un" v-model="form.username" required :aria-invalid="!!formError?.fieldError('username')" />
          <span v-if="formError?.fieldError('username')" class="error">{{ formError.fieldError('username') }}</span>
        </div>
        <div class="field"><label for="dn">姓名</label><input id="dn" v-model="form.display_name" required /></div>
        <div class="field">
          <label for="pw">初始密碼</label>
          <input id="pw" v-model="form.password" type="password" required :aria-invalid="!!formError?.fieldError('password')" />
          <span v-if="formError?.fieldError('password')" class="error">{{ formError.fieldError('password') }}</span>
          <span v-else class="hint">至少 10 個字元。</span>
        </div>
      </div>

      <div class="field">
        <label>角色</label>
        <div class="btn-row">
          <label
            v-for="r in roles" :key="r.code"
            class="tag" style="cursor: pointer; font-weight: 500"
            :style="form.roles.includes(r.code)
              ? 'background:#f2f7fa;border-color:#c8d8e2;color:var(--ink)'
              : 'background:#fff;border-color:var(--line);color:var(--text-muted)'"
          >
            <input v-model="form.roles" type="checkbox" :value="r.code" style="width: auto; margin-right: 6px" />
            {{ r.name }}
          </label>
        </div>
        <span v-if="formError?.fieldError('roles')" class="error">{{ formError.fieldError('roles') }}</span>
      </div>

      <button class="btn" :disabled="busy">{{ busy ? '建立中…' : '建立' }}</button>
    </form>
  </div>

  <div class="card">
    <table>
      <thead>
        <tr><th>帳號</th><th>姓名</th><th>角色</th><th>狀態</th><th></th></tr>
      </thead>
      <tbody>
        <tr v-for="u in users" :key="u.id">
          <td class="code">{{ u.username }}</td>
          <td>{{ u.display_name }}</td>
          <td>
            <span v-if="!u.roles.length" class="faint">未指派</span>
            <span v-for="r in u.roles" :key="r" class="tag lead" style="margin-right: 4px">{{ roleName(r) }}</span>
          </td>
          <td>
            <span class="tag" :class="u.is_active ? 'approved' : 'superseded'">
              {{ u.is_active ? '啟用中' : '已停用' }}
            </span>
          </td>
          <td>
            <button class="btn ghost sm" @click="toggleActive(u)">{{ u.is_active ? '停用' : '啟用' }}</button>
          </td>
        </tr>
      </tbody>
    </table>
    <div v-if="loading" class="empty">載入中…</div>
  </div>

  <div class="card">
    <div class="card-head"><h2>角色與權限</h2></div>
    <table>
      <thead><tr><th>角色</th><th>說明</th><th>權限數</th></tr></thead>
      <tbody>
        <tr v-for="r in roles" :key="r.code">
          <td>{{ r.name }}<span v-if="r.is_system" class="faint">　系統預設</span></td>
          <td class="muted">{{ r.description ?? '—' }}</td>
          <td class="num">{{ r.permissions.length }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
