import { ref, computed, unref } from 'vue'
import { simulateBudget } from '@/utils/governanceStatus'

/**
 * Interactive budget-simulation state layered on the shared simulateBudget engine.
 * Selecting proposals only changes the candidate pool; net-vote rank + the cap still
 * decide what gets funded -- so the sandbox always obeys the real protocol rule.
 *
 * @param {import('vue').Ref<Array>|Function} candidatesRef - selectable proposals (active)
 * @param {import('vue').Ref<Array>|Function} actualFundedRef - proposals funded right now (preload / reset target)
 */
export function useBudgetSimulation(candidatesRef, actualFundedRef) {
  // Accept either a ref/computed or a plain getter function for each source.
  const resolve = (src) => (typeof src === 'function' ? src() : unref(src))
  const candidates = computed(() => resolve(candidatesRef) || [])
  const actualFunded = computed(() => resolve(actualFundedRef) || [])
  const actualFundedHashes = computed(() => new Set(actualFunded.value.map(p => p.Hash)))

  // Selected proposal Hashes. Reassigned on every change (not mutated in place) so
  // Vue's reactivity tracks Set updates.
  const selected = ref(new Set())

  const isSelected = (hash) => selected.value.has(hash)

  const toggle = (hash) => {
    const next = new Set(selected.value)
    if (next.has(hash)) next.delete(hash)
    else next.add(hash)
    selected.value = next
  }

  const selectAll = () => {
    selected.value = new Set(candidates.value.map(p => p.Hash))
  }

  const clear = () => {
    selected.value = new Set()
  }

  /** Re-seed the selection from today's actually-funded set. */
  const resetToActual = () => {
    selected.value = new Set(actualFundedHashes.value)
  }

  const selectedProposals = computed(() =>
    candidates.value.filter(p => selected.value.has(p.Hash))
  )

  // The cap-limited, vote-ranked allocation over the selected set.
  const simulation = computed(() => simulateBudget(selectedProposals.value))

  // Hash -> { funded, cumulative } for the selected (ranked) proposals.
  const simByHash = computed(() => {
    const m = new Map()
    for (const r of simulation.value.ranked) m.set(r.proposal.Hash, r)
    return m
  })

  // Diff vs today's actual funded set (the insight the preload makes possible).
  const delta = computed(() => {
    const actual = actualFundedHashes.value
    const sel = selected.value
    let added = 0
    let removed = 0
    sel.forEach(h => { if (!actual.has(h)) added++ })
    actual.forEach(h => { if (!sel.has(h)) removed++ })
    return { added, removed }
  })

  const selectedCount = computed(() => selected.value.size)

  return {
    selected,
    selectedCount,
    isSelected,
    toggle,
    selectAll,
    clear,
    resetToActual,
    selectedProposals,
    simulation,
    simByHash,
    delta,
    actualFundedHashes,
  }
}
