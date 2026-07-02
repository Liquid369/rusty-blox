<script setup>
/* Framed glass HUD panel with corner reticles + a telemetry header bar. */
defineProps({
  title: { type: String, default: '' },
  id: { type: String, default: '' },
  hero: { type: Boolean, default: false },
  pad: { type: Boolean, default: false },
})
</script>

<template>
  <section class="panel" :class="{ hero }">
    <header v-if="title" class="panel-head">
      <span class="dot neon"></span>
      <span class="ttl">{{ title }}</span>
      <slot name="head" />
      <span v-if="id" class="id">{{ id }}</span>
    </header>
    <div :class="pad ? 'panel-body pad-x' : 'panel-body'">
      <slot />
    </div>
  </section>
</template>

<style scoped>
.pad-x { padding: var(--space-5); }
</style>
