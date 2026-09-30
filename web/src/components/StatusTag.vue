<script setup lang="ts">
const LABELS: Record<string, string> = {
  draft: '草稿',
  pending: '送審中',
  approved: '已核准',
  rejected: '已駁回',
  superseded: '已被取代',
  lead: '潛在',
  quoting: '報價中',
  contracted: '已簽約',
  closed: '已結案',
  cancelled: '已取消',
}

const props = defineProps<{ status: string }>()
</script>

<template>
  <span class="tag" :class="props.status">{{ LABELS[props.status] ?? props.status }}</span>
</template>
