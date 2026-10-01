<script setup lang="ts">
/**
 * 列表分頁。
 *
 * 存在的理由不是 UX 而是正確性：列表原本硬寫 `limit=200`，超過之後
 * 就靜默看不到剩下的資料。數字全部來自後端回傳的 total / limit / offset，
 * 前端不自行推算。
 */
const props = defineProps<{ total: number; limit: number; offset: number; busy?: boolean }>()
const emit = defineEmits<{ go: [offset: number] }>()

const from = () => (props.total === 0 ? 0 : props.offset + 1)
const to = () => Math.min(props.offset + props.limit, props.total)
const hasPrev = () => props.offset > 0
const hasNext = () => props.offset + props.limit < props.total
</script>

<template>
  <div v-if="total > limit" class="pager">
    <button class="btn ghost sm" :disabled="busy || !hasPrev()" @click="emit('go', Math.max(0, offset - limit))">
      上一頁
    </button>
    <span class="faint">第 {{ from() }}–{{ to() }} 筆，共 {{ total }} 筆</span>
    <button class="btn ghost sm" :disabled="busy || !hasNext()" @click="emit('go', offset + limit)">
      下一頁
    </button>
  </div>
</template>

<style scoped>
.pager {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 0;
}
</style>
