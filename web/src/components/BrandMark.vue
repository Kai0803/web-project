<script setup lang="ts">
/** 品牌標記（彎頭）。深色底時管身轉白，管口的銅色維持不變。 */
defineProps<{ onDark?: boolean }>()
</script>

<template>
  <svg viewBox="0 0 100 100" aria-hidden="true">
    <path
      d="M 20 80 H 40 A 40 40 0 0 0 80 40 V 20"
      fill="none"
      :stroke="onDark ? '#ffffff' : '#1F3A4D'"
      stroke-width="19"
    />
    <path d="M 23 70.5 V 89.5" stroke="#B87333" stroke-width="6" />
    <path d="M 70.5 23 H 89.5" stroke="#B87333" stroke-width="6" />
  </svg>
</template>
