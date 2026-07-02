<script setup>
/* Glowing telemetry stat tile. */
defineProps({
  k: { type: String, default: '' },     // label
  v: { type: [String, Number], default: '—' }, // value
  s: { type: String, default: '' },     // sub
  accent: { type: Boolean, default: false },
  glow: { type: Boolean, default: false },
  live: { type: Boolean, default: false },
})
</script>

<template>
  <div class="stat" :class="{ accent }">
    <div class="k">
      <span v-if="live" class="dot live"></span>{{ k }}
    </div>
    <div class="v" :class="{ glow }">
      <slot name="v">{{ v }}</slot>
    </div>
    <div v-if="s || $slots.s" class="s"><slot name="s">{{ s }}</slot></div>
  </div>
</template>
