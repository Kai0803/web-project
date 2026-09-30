<script setup lang="ts">
import { ApiError } from '@/api/client'

const props = defineProps<{ error: unknown }>()

/** 只顯示訊息本身。後端已確保內部錯誤不外洩細節，僅附追蹤 ID。 */
function message(err: unknown): string {
  if (err instanceof ApiError) return err.message
  if (err instanceof Error) return err.message
  return '發生未預期的錯誤'
}
</script>

<template>
  <div v-if="props.error" class="alert error" role="alert">{{ message(props.error) }}</div>
</template>
