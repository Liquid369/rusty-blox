<script setup>
/* =====================================================================
   TRANSACTION DETAIL — centerpiece value-flow Sankey (vin → vout) with
   spent/unspent state pills. UNITS: EVERY money field is a satoshi
   STRING → formatSats (BigInt-safe). spent: true/false/null(unknown).
   ===================================================================== */
import { ref, watch, onMounted, computed } from 'vue'
import { getTx, getBlockDetail, getBudgetInfo, getFinalizedBudgets } from '../api/client.js'
import { formatSats, formatPiv } from '../lib/money.js'
import { formatDateTime, truncateHash, formatCount, timeAgo } from '../lib/format.js'
import { baseOption, palette, hexA } from '../lib/chart.js'
import { isCoinstakeTx, isUnresolvedColdVin, coinstakeInputAddresses, coinstakeInputValueSat } from '../lib/coinstake.js'
import EChart from '../components/EChart.vue'
import HudPanel from '../components/HudPanel.vue'
import Stat from '../components/Stat.vue'
import Copyable from '../components/Copyable.vue'

const props = defineProps({ txid: { type: String, required: true } })
const tx = ref(null)
const err = ref(null)
// Block reward (PIV float) for a cold-stake coinstake whose inputs the backend left
// unresolved; /tx carries no reward, so we fetch it from the owning block.
const blockReward = ref(null)

async function load() {
  err.value = null; tx.value = null; blockReward.value = null
  try {
    const t = await getTx(props.txid)
    tx.value = t
    if (isCoinstakeTx(t) && (t.vin || []).some(isUnresolvedColdVin) && t.blockHeight != null) {
      try { blockReward.value = (await getBlockDetail(t.blockHeight)).reward } catch { /* value falls back to — */ }
    }
  } catch (e) { err.value = e.message }
}
onMounted(load)
watch(() => props.txid, load)

// [staker(S), owner(D)] to show for an unresolved cold-stake input, else null.
const vinCold = (vin) => coinstakeInputAddresses(tx.value, vin)
// Consumed stake (satoshi) for the single input of a cold coinstake, else null.
const coldInputValueSat = computed(() => {
  const t = tx.value
  if (!t) return null
  const rewardSat = blockReward.value != null ? Math.round(blockReward.value * 1e8) : null
  // Pass the satoshi STRING (not Number(t.value)) so BigInt keeps full precision.
  return coinstakeInputValueSat(t.value, rewardSat, (t.vin || []).length)
})

// --- Sapling / shielded --------------------------------------------------
const sapling = computed(() => tx.value?.sapling || null)
const isShielded = computed(() => {
  const s = sapling.value
  return !!s && ((s.shielded_spend_count || 0) > 0 || (s.shielded_output_count || 0) > 0)
})
// value_balance is a signed PIV FLOAT (not satoshis, so no formatSats): the
// backend sets it > 0 for unshielding (shield -> transparent), < 0 for shielding
// (transparent -> shield), 0 for a pure shielded (z->z) transfer.
const shieldDirection = computed(() => {
  const vb = sapling.value?.value_balance
  if (vb == null) return null
  if (vb < 0) return 'Shielding'
  if (vb > 0) return 'Deshielding'
  return 'Shielded transfer'
})
const valueBalance = computed(() => {
  const vb = sapling.value?.value_balance
  if (vb == null) return '—'
  const n = Number(vb)
  return (n > 0 ? '+' : '') + n.toFixed(8)
})

// Decode a vout scriptPubKey (the /tx API gives per-output `hex` but no type)
// into a coarse type + OP_RETURN payload. Handles direct pushes + PUSHDATA1/2.
function voutScript(vout) {
  const h = (vout.hex || '').toLowerCase()
  if (h.startsWith('6a')) {
    let i = 2
    const parts = []
    while (i + 2 <= h.length) {
      const op = parseInt(h.slice(i, i + 2), 16); i += 2
      let n = 0
      if (op <= 0x4b) n = op
      else if (op === 0x4c) { n = parseInt(h.slice(i, i + 2), 16); i += 2 }
      else if (op === 0x4d) { n = parseInt(h.slice(i + 2, i + 4) + h.slice(i, i + 2), 16); i += 4 }
      else break
      parts.push(h.slice(i, i + n * 2)); i += n * 2
    }
    return { type: 'OP_RETURN', data: parts.join('') }
  }
  if (h.startsWith('76a914') && h.endsWith('88ac')) return { type: 'P2PKH' }
  if (h.startsWith('a914') && h.endsWith('87')) return { type: 'P2SH' }
  return { type: vout.addresses && vout.addresses.length ? 'address' : 'nonstandard' }
}
// hex -> ASCII (· for non-printable); '' when nothing printable (pure binary blob).
function hexToAscii(hex) {
  let s = '', printable = 0
  for (let i = 0; i + 2 <= hex.length; i += 2) {
    const c = parseInt(hex.slice(i, i + 2), 16)
    if (c >= 32 && c < 127) { s += String.fromCharCode(c); printable++ } else s += '·'
  }
  return printable ? s : ''
}
// vout decorated with its decoded script, so the template parses each hex once.
// Detect a PIVX budget collateral OP_RETURN: a 32-byte hash (6a 20) whose output
// burns the fee — >= 50 PIV = proposal (PROPOSAL_FEE_TX), 5..49 PIV = finalization
// (BUDGET_FEE_TX). Confirmed against Core's CheckCollateral. `value` is a satoshi
// STRING, so compare with BigInt (never Number() a satoshi field).
function budgetCollateral(v) {
  if (v.script.type !== 'OP_RETURN' || (v.script.data || '').length !== 64) return null
  let sats
  try { sats = BigInt(v.value || '0') } catch { return null }
  if (sats >= 5_000_000_000n) return { kind: 'Budget proposal', hash: v.script.data }
  if (sats >= 500_000_000n) return { kind: 'Budget finalization', hash: v.script.data }
  return null
}
const vouts = computed(() => (tx.value?.vout || []).map((v) => {
  const out = { ...v, script: voutScript(v) }
  out.budget = budgetCollateral(out)
  return out
}))
const txBudget = computed(() => vouts.value.find((v) => v.budget)?.budget || null)

// Resolve a budget collateral tx to its governance record: proposals match by
// FeeHash == txid (getbudgetinfo), finalized budgets by FeeTX == txid
// (getfinalizedbudgets). The node prunes old budgets, so it may be unresolvable.
const govRecord = ref(null)
async function resolveGovernance() {
  govRecord.value = null
  const b = txBudget.value
  const id = (tx.value?.txid || '').toLowerCase()
  if (!b || !id) return
  try {
    if (b.kind === 'Budget proposal') {
      const props = await getBudgetInfo()
      const p = (props || []).find((x) => (x.FeeHash || '').toLowerCase() === id)
      if (p) govRecord.value = { kind: 'proposal', ...p }
    } else {
      const fbs = await getFinalizedBudgets()
      for (const [name, fb] of Object.entries(fbs || {})) {
        if ((fb.FeeTX || '').toLowerCase() === id) { govRecord.value = { kind: 'finalized', name, ...fb }; break }
      }
    }
  } catch { /* leave unresolved */ }
}
watch(txBudget, resolveGovernance)
// Full /tx response, pretty-printed, for the copyable raw-JSON section.
const rawJson = computed(() => (tx.value ? JSON.stringify(tx.value, null, 2) : ''))

function spentPill(v) {
  if (v === true) return { cls: 'bad', text: 'spent' }
  if (v === false) return { cls: 'ok', text: 'unspent' }
  return { cls: 'warn', text: 'unknown' }
}
// satoshi string -> PIV number (display only; never used for money text)
const sat2piv = (v) => parseFloat(formatSats(v, { decimals: 8, group: false })) || 0

/* ---------- value-flow Sankey ---------- */
const sankeyOption = computed(() => {
  const p = palette()
  if (!tx.value) return baseOption(p)
  const t = tx.value
  const nodes = []
  const links = []
  const seen = new Map()
  const node = (id, side) => {
    if (!seen.has(id)) {
      seen.set(id, true)
      nodes.push({ name: id, itemStyle: { color: side === 'in' ? p.cyan : p.neon, borderColor: 'rgba(7,4,13,0.6)' } })
    }
    return id
  }
  const TXC = '◇ TX'
  nodes.push({ name: TXC, itemStyle: { color: p.amber }, label: { color: p.amber, fontWeight: 700 } })

  // For a cold-stake (P2CS) input/output, addresses = [staker(S), owner(D)] — the
  // OWNER is the economic party in the flow, so label the node with the last address.
  t.vin.forEach((vin, i) => {
    // Recover the cold-staker for unresolved P2CS coinstake inputs (backend blanks them).
    const cold = vinCold(vin)
    const a = cold || vin.addresses || []
    const addr = a.length ? truncateHash(a[a.length - 1], 6, 5) : `coinbase#${i}`
    const v = cold ? (coldInputValueSat.value || 0) / 1e8 : sat2piv(vin.value)
    if (v > 0) links.push({ source: node(`in:${addr}`, 'in'), target: TXC, value: v })
  })
  t.vout.forEach((vout, i) => {
    const a = vout.addresses || []
    const addr = a.length ? truncateHash(a[a.length - 1], 6, 5) : `out#${i}`
    const v = sat2piv(vout.value)
    if (v > 0) links.push({ source: TXC, target: node(`out:${addr}`, 'out'), value: v })
  })

  // Sapling: draw the net value crossing the shielded-pool boundary as its own
  // node. value_balance (PIV, signed): < 0 shielding (TX -> pool), > 0
  // deshielding (pool -> TX); 0 (pure z->z) has no transparent flow to show.
  const vb = t.sapling ? Number(t.sapling.value_balance) : 0
  if (vb) {
    const POOL = '◈ SHIELDED'
    nodes.push({ name: POOL, itemStyle: { color: '#8f5cff' }, label: { color: '#c9a6ff', fontWeight: 700 } })
    if (vb < 0) links.push({ source: TXC, target: POOL, value: Math.abs(vb) })
    else links.push({ source: POOL, target: TXC, value: vb })
  }

  return {
    backgroundColor: 'transparent',
    tooltip: { ...baseOption(p).tooltip, trigger: 'item', formatter: (d) => d.dataType === 'edge' ? `${d.value.toFixed(4)} PIV` : d.name.replace(/^(in|out):/, '') },
    series: [{
      type: 'sankey', left: 14, right: 104, top: 16, bottom: 16,
      nodeWidth: 14, nodeGap: 12,
      data: nodes, links,
      label: { color: p.text, fontFamily: 'monospace', fontSize: 10.5, overflow: 'none', formatter: (d) => d.name.replace(/^(in|out):/, '') },
      lineStyle: { color: 'gradient', opacity: 0.32, curveness: 0.5 },
      itemStyle: { borderWidth: 1 },
      emphasis: { focus: 'adjacency', lineStyle: { opacity: 0.55 } },
    }],
  }
})

// The Sankey needs ~20px of vertical room per node or the labels collapse into an
// illegible block (a 44-output payout crammed into a fixed 300px). Scale to the busier
// side. ponytail: capped at 1200px — a pathological fan-out (>~58 nodes) re-crowds past
// the cap, acceptable ceiling; raise it or aggregate small outputs if that ever matters.
const flowHeight = computed(() => {
  const n = Math.max(tx.value?.vin?.length || 1, tx.value?.vout?.length || 1)
  return Math.min(1200, Math.max(300, n * 20 + 40)) + 'px'
})
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <div class="eyebrow">CORE EXPLORER · /tx</div>
        <h1 class="page-title">Transaction</h1>
      </div>
    </div>

    <div v-if="err" class="banner bad">{{ err }}</div>

    <template v-if="tx">
      <HudPanel class="txid-panel">
        <div class="txid-row">
          <span class="pill mono" :class="tx.confirmations > 0 ? 'neon' : 'warn'"><span class="dot" :class="tx.confirmations > 0 ? 'neon' : 'warn'"></span>{{ tx.confirmations > 0 ? 'CONFIRMED' : 'UNCONFIRMED' }}</span>
          <span v-if="isShielded" class="pill cyan mono"><span class="dot cyan"></span>SHIELDED{{ shieldDirection ? ' · ' + shieldDirection.toUpperCase() : '' }}</span>
          <span v-if="txBudget" class="pill neon mono"><span class="dot neon"></span>{{ txBudget.kind.toUpperCase() }}</span>
          <span class="mono txid-val">{{ tx.txid }}</span>
        </div>
      </HudPanel>

      <div class="statgrid cols-4" style="margin-top: var(--space-4)">
        <Stat k="VALUE OUT" accent><template #v>{{ formatSats(tx.value, { decimals: 4 }) }}</template><template #s>PIV</template></Stat>
        <Stat k="VALUE IN"><template #v>{{ formatSats(coldInputValueSat != null ? coldInputValueSat : tx.valueIn, { decimals: 4 }) }}</template><template #s>PIV</template></Stat>
        <Stat k="FEES" glow><template #v>{{ formatSats(tx.fees) }}</template><template #s>PIV</template></Stat>
        <Stat k="CONFIRMATIONS" live><template #v>{{ formatCount(tx.confirmations) }}</template><template #s>{{ timeAgo(tx.blockTime) }}</template></Stat>
      </div>

      <h2 class="section-title">Value flow</h2>
      <HudPanel title="VIN → VOUT VALUE-FLOW" :id="isShielded ? 'transparent + shielded pool' : 'sankey · satoshi → PIV'" hero>
        <template #head><span class="pill cyan mono">{{ tx.vin.length }} in</span><span class="pill neon mono">{{ tx.vout.length }} out</span></template>
        <EChart :option="sankeyOption" :height="flowHeight" aria-label="Transaction value flow from inputs to outputs" />
      </HudPanel>

      <div class="split s-2" style="margin-top: var(--space-4)">
        <HudPanel title="INPUTS" :id="`${tx.vin.length} vin`">
          <table class="dtable">
            <thead><tr><th>Address</th><th class="num">Value (PIV)</th></tr></thead>
            <tbody>
              <tr v-for="(vin, i) in tx.vin" :key="i">
                <td>
                  <template v-if="vinCold(vin)">
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vinCold(vin)[1]}`">{{ truncateHash(vinCold(vin)[1], 8, 6) }}</RouterLink><span class="pill neon mono">OWNER</span></div>
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vinCold(vin)[0]}`">{{ truncateHash(vinCold(vin)[0], 8, 6) }}</RouterLink><span class="pill cyan mono">STAKER</span></div>
                  </template>
                  <template v-else-if="vin.addresses && vin.addresses.length >= 2">
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vin.addresses[1]}`">{{ truncateHash(vin.addresses[1], 8, 6) }}</RouterLink><span class="pill neon mono">OWNER</span></div>
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vin.addresses[0]}`">{{ truncateHash(vin.addresses[0], 8, 6) }}</RouterLink><span class="pill cyan mono">STAKER</span></div>
                  </template>
                  <RouterLink v-else-if="vin.addresses && vin.addresses[0]" :to="`/address/${vin.addresses[0]}`">{{ truncateHash(vin.addresses[0], 10, 8) }}</RouterLink>
                  <RouterLink v-else-if="vin.txid" :to="`/tx/${vin.txid}`" class="dim mono" :title="`${vin.txid}:${vin.vout}`">{{ truncateHash(vin.txid, 8, 6) }}:{{ vin.vout }}</RouterLink>
                  <span v-else class="dim">coinbase</span>
                </td>
                <td class="num strong">{{ vinCold(vin) ? (coldInputValueSat != null ? formatSats(coldInputValueSat, { decimals: 4 }) : '—') : (vin.value != null ? formatSats(vin.value, { decimals: 4 }) : '—') }}</td>
              </tr>
            </tbody>
          </table>
        </HudPanel>

        <HudPanel title="OUTPUTS" :id="`${tx.vout.length} vout`">
          <div style="overflow-x:auto">
          <table class="dtable">
            <thead><tr><th>Address</th><th class="num">Value (PIV)</th><th>State</th></tr></thead>
            <tbody>
              <tr v-for="(vout, i) in vouts" :key="i">
                <td>
                  <template v-if="vout.addresses && vout.addresses.length >= 2">
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vout.addresses[1]}`">{{ truncateHash(vout.addresses[1], 8, 6) }}</RouterLink><span class="pill neon mono">OWNER</span></div>
                    <div style="display:flex;align-items:center;gap:6px;margin:1px 0"><RouterLink :to="`/address/${vout.addresses[0]}`">{{ truncateHash(vout.addresses[0], 8, 6) }}</RouterLink><span class="pill cyan mono">STAKER</span></div>
                  </template>
                  <RouterLink v-else-if="vout.addresses && vout.addresses[0]" :to="`/address/${vout.addresses[0]}`">{{ truncateHash(vout.addresses[0], 10, 8) }}</RouterLink>
                  <template v-else-if="vout.script.type === 'OP_RETURN'">
                    <span v-if="vout.budget" class="pill neon mono">{{ vout.budget.kind.toUpperCase() }}</span>
                    <span v-else class="pill warn mono">OP_RETURN</span>
                    <div style="margin-top:4px"><Copyable :value="vout.script.data">{{ truncateHash(vout.script.data, 14, 12) }}</Copyable></div>
                    <div v-if="vout.budget" class="dim mono" style="font-size:11px;margin-top:2px">budget hash · {{ formatSats(vout.value, { decimals: 0 }) }} PIV fee (burned)</div>
                    <div v-else-if="hexToAscii(vout.script.data)" class="dim mono" style="font-size:11px;margin-top:2px">“{{ hexToAscii(vout.script.data) }}”</div>
                  </template>
                  <span v-else class="dim mono">{{ vout.script.type.toLowerCase() }}</span>
                </td>
                <td class="num strong">{{ formatSats(vout.value, { decimals: 4 }) }}</td>
                <td><span class="pill" :class="spentPill(vout.spent).cls">{{ spentPill(vout.spent).text }}</span></td>
              </tr>
            </tbody>
          </table>
          </div>
        </HudPanel>
      </div>

      <template v-if="isShielded">
        <h2 class="section-title">Shielded (Sapling)</h2>
        <HudPanel title="SAPLING SHIELDED" :id="shieldDirection || 'shielded'">
          <template #head>
            <span class="pill cyan mono">{{ formatCount(sapling.shielded_spend_count) }} spend{{ sapling.shielded_spend_count === 1 ? '' : 's' }}</span>
            <span class="pill neon mono">{{ formatCount(sapling.shielded_output_count) }} output{{ sapling.shielded_output_count === 1 ? '' : 's' }}</span>
          </template>
          <div class="statgrid cols-3">
            <Stat k="VALUE BALANCE" accent><template #v>{{ valueBalance }}</template><template #s>PIV · {{ shieldDirection }}</template></Stat>
            <Stat k="SHIELDED SPENDS"><template #v>{{ formatCount(sapling.shielded_spend_count) }}</template><template #s>notes consumed</template></Stat>
            <Stat k="SHIELDED OUTPUTS"><template #v>{{ formatCount(sapling.shielded_output_count) }}</template><template #s>notes created</template></Stat>
          </div>
          <dl class="kv" style="margin-top: var(--space-4)">
            <dt>Binding signature</dt>
            <dd><Copyable v-if="sapling.binding_sig" :value="sapling.binding_sig">{{ truncateHash(sapling.binding_sig, 18, 14) }}</Copyable><span v-else class="dim">—</span></dd>
          </dl>
          <div class="split s-2" style="margin-top: var(--space-4)">
            <div v-if="sapling.spends && sapling.spends.length" style="overflow-x:auto">
              <div class="mono dim" style="margin-bottom:6px">SHIELDED SPENDS · {{ sapling.spends.length }} · click a value to copy</div>
              <table class="dtable">
                <thead><tr><th>Nullifier</th><th>Anchor</th><th>Value commitment</th></tr></thead>
                <tbody>
                  <tr v-for="(s, i) in sapling.spends" :key="i">
                    <td><Copyable :value="s.nullifier">{{ truncateHash(s.nullifier, 8, 6) }}</Copyable></td>
                    <td><Copyable :value="s.anchor">{{ truncateHash(s.anchor, 8, 6) }}</Copyable></td>
                    <td><Copyable :value="s.cv">{{ truncateHash(s.cv, 8, 6) }}</Copyable></td>
                  </tr>
                </tbody>
              </table>
            </div>
            <div v-if="sapling.outputs && sapling.outputs.length" style="overflow-x:auto">
              <div class="mono dim" style="margin-bottom:6px">SHIELDED OUTPUTS · {{ sapling.outputs.length }} · click a value to copy</div>
              <table class="dtable">
                <thead><tr><th>Commitment</th><th>Ephemeral key</th><th>Value commitment</th></tr></thead>
                <tbody>
                  <tr v-for="(o, i) in sapling.outputs" :key="i">
                    <td><Copyable :value="o.cmu">{{ truncateHash(o.cmu, 8, 6) }}</Copyable></td>
                    <td><Copyable :value="o.ephemeral_key">{{ truncateHash(o.ephemeral_key, 8, 6) }}</Copyable></td>
                    <td><Copyable :value="o.cv">{{ truncateHash(o.cv, 8, 6) }}</Copyable></td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
          <p class="dim" style="margin-top:var(--space-3);font-size:12px">
            Shielded addresses and amounts are private by design — only the public commitments, nullifiers, and the net transparent value balance are visible on-chain. Click any value to copy the full hex.
          </p>
        </HudPanel>
      </template>

      <template v-if="txBudget">
        <h2 class="section-title">Governance</h2>
        <HudPanel :title="txBudget.kind === 'Budget finalization' ? 'FINALIZED BUDGET' : 'BUDGET PROPOSAL'" id="decoded from collateral">
          <template v-if="govRecord && govRecord.kind === 'proposal'">
            <div class="statgrid cols-4">
              <Stat k="PROPOSAL" accent><template #v>{{ govRecord.Name }}</template><template #s>{{ govRecord.IsValid ? 'valid' : 'invalid' }}</template></Stat>
              <Stat k="MONTHLY"><template #v>{{ formatPiv(govRecord.MonthlyPayment, { decimals: 0 }) }}</template><template #s>PIV · {{ govRecord.TotalPaymentCount }} payments</template></Stat>
              <Stat k="VOTES"><template #v>{{ govRecord.Yeas }} / {{ govRecord.Nays }}</template><template #s>yea / nay</template></Stat>
              <Stat k="PAYS"><template #v>#{{ govRecord.BlockStart }}</template><template #s>→ #{{ govRecord.BlockEnd }}</template></Stat>
            </div>
            <dl class="kv" style="margin-top:var(--space-4)">
              <dt>URL</dt><dd><a :href="govRecord.URL" target="_blank" rel="noopener noreferrer">{{ govRecord.URL }}</a></dd>
              <dt>Payee</dt><dd><RouterLink :to="`/address/${govRecord.PaymentAddress}`">{{ govRecord.PaymentAddress }}</RouterLink></dd>
            </dl>
          </template>
          <template v-else-if="govRecord && govRecord.kind === 'finalized'">
            <div class="statgrid cols-4">
              <Stat k="STATUS" accent><template #v>{{ govRecord.Status }}</template><template #s>{{ govRecord.IsValid ? 'valid' : 'invalid' }}</template></Stat>
              <Stat k="VOTES"><template #v>{{ formatCount(govRecord.VoteCount) }}</template><template #s>masternode votes</template></Stat>
              <Stat k="PAYS"><template #v>#{{ govRecord.BlockStart }}</template><template #s>→ #{{ govRecord.BlockEnd }}</template></Stat>
              <Stat k="PROPOSALS"><template #v>{{ (govRecord.Proposals || '').split(',').filter(Boolean).length || '—' }}</template><template #s>in this budget</template></Stat>
            </div>
            <dl class="kv" style="margin-top:var(--space-4)">
              <dt>Budget</dt><dd class="mono">{{ govRecord.name }}</dd>
              <dt>Proposals</dt><dd class="mono">{{ govRecord.Proposals || '—' }}</dd>
            </dl>
          </template>
          <div v-else class="dim">
            Collateral hash <span class="mono">{{ truncateHash(txBudget.hash, 12, 10) }}</span> — the node no longer tracks this {{ txBudget.kind.toLowerCase().replace('budget ', '') }} (old budgets are pruned), so its contents can't be resolved.
          </div>
        </HudPanel>
      </template>

      <h2 class="section-title">Context</h2>
      <HudPanel title="LEDGER CONTEXT" id="/tx metadata">
        <dl class="kv">
          <dt>Block height</dt><dd><RouterLink :to="`/block/${tx.blockHeight}`">#{{ tx.blockHeight }}</RouterLink></dd>
          <dt>Block hash</dt><dd>{{ truncateHash(tx.blockHash, 18, 14) }}</dd>
          <dt>Block time</dt><dd>{{ formatDateTime(tx.blockTime) }}</dd>
          <dt>Size</dt><dd>{{ formatCount(tx.size) }} B · vsize {{ formatCount(tx.vsize) }} B</dd>
          <dt>Version</dt><dd>{{ tx.version }} · locktime {{ tx.lockTime }}</dd>
        </dl>
      </HudPanel>

      <h2 class="section-title">Raw</h2>
      <HudPanel title="RAW TRANSACTION JSON" id="/tx response">
        <template #head><Copyable :value="rawJson"><span class="pill cyan mono">⧉ copy JSON</span></Copyable></template>
        <details>
          <summary class="mono dim" style="cursor:pointer">show / hide the full /tx response</summary>
          <pre style="max-width:100%;margin-top:10px;padding:12px;font-size:11px;line-height:1.55;white-space:pre-wrap;overflow-wrap:anywhere;color:var(--text-muted);background:rgba(0,0,0,0.25);border-radius:8px">{{ rawJson }}</pre>
        </details>
      </HudPanel>
    </template>

    <div v-else-if="!err" class="sk" style="height: 200px"></div>
  </div>
</template>

<style scoped>
.txid-panel { margin-top: 0; }
.txid-row { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
.txid-val { font-size: 12.5px; color: var(--text-muted); word-break: break-all; }
</style>
