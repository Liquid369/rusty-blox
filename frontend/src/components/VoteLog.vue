<script setup>
/* Vote log: budget votes the sidecar read from the node's debug.log, with the
   masternode behind each rejected vote where the vote hash allows recovering it.
   How a masternode was found is shown per row; the methods are different
   strengths of evidence and are never blended. */
import { ref, computed, onMounted, h } from 'vue'
import { RouterLink } from 'vue-router'
import { getGovLog, getGovGroups } from '../api/client.js'
import { formatCount, formatDateTime, truncateHash } from '../lib/format.js'
import { baseOption, palette } from '../lib/chart.js'
import EChart from './EChart.vue'
import HudPanel from './HudPanel.vue'
import Stat from './Stat.vue'

const hours = ref(24)
const data = ref(null)
const error = ref(null)
const loading = ref(false)
let req = 0

/* Groups come from governance history, not the sidecar: either can fail alone. */
const groups = ref(null)
async function load() {
  const id = ++req
  loading.value = true
  error.value = null
  // Cleared with the window, so a prior window's groups never show beside a new error.
  groups.value = null
  getGovGroups(hours.value)
    .then((g) => { if (id === req) groups.value = g })
    .catch(() => { if (id === req) groups.value = null })
  try {
    const d = await getGovLog(hours.value)
    if (id === req) data.value = d
  } catch (e) {
    if (id === req) { data.value = null; error.value = e.message || 'Could not load the vote log' }
  } finally {
    if (id === req) loading.value = false
  }
}

function span(secs) {
  if (secs < 60) return `${secs} s`
  if (secs < 3600) return `${Math.floor(secs / 60)} min ${secs % 60} s`
  return `${Math.floor(secs / 3600)} h ${Math.floor((secs % 3600) / 60)} min`
}
const share = (n) => {
  const e = groups.value?.enabled_masternodes
  return e ? `${((100 * n) / e).toFixed(1)}% of ${formatCount(e)} enabled` : ''
}
const signed = (n) => (n > 0 ? `+${n}` : `${n}`)
onMounted(load)
const setHours = (h) => { hours.value = h; load() }

const CHOICE = ['abstain', 'yes', 'no']

/* Reasons and evidence share one vocabulary across chart, tags and key. */
const REASONS = {
  'signed over 1 h in the future': { short: 'Future-dated', tone: 'hot' },
  'older than the stored vote': { short: 'Older than stored', tone: 'neon' },
  'under 1 h after the stored vote': { short: 'Too soon', tone: 'amber' },
}
const reasonOf = (r) => REASONS[r] || { short: r, tone: '' }

/* Ordered strongest first; the legend renders in this order. Colour is
   reserved for verified evidence; unverified evidence gets a dashed outline,
   so it never shares a colour with the rejection reasons. */
const METHODS = {
  matched: { label: 'Exact match', tone: 'cyan', what: 'Signed by this masternode: the hash rebuilds and the node verified the signature.' },
  known: { label: 'Replay', tone: 'rose', what: 'This masternode’s own earlier vote, resent by someone else.' },
  claimed: { label: 'Claimed only', tone: 'unverified', what: 'The vote names this collateral, but its signature was never checked.' },
  prefix: { label: 'Prefix match', tone: 'unverified', what: 'Only the first 10 characters of the collateral were logged.' },
  not_identifiable: { label: 'Not traced', tone: 'none', what: 'No known collateral rebuilds this hash.' },
}
const methodOf = (r) => {
  if (r.mn.method === 'matched' && !r.mn.signed) return METHODS.claimed
  const m = METHODS[r.mn.method] || METHODS.not_identifiable
  if (r.mn.method === 'known' && !r.reason.startsWith('older')) return { ...m, label: 'Known vote', tone: 'cyan' }
  return m
}

/* Plain words for rejections Core logs without a vote hash. */
function plainOther(text) {
  if (/-sig:/.test(text)) return 'Invalid signature'
  if (/unknown masternode/.test(text)) return 'Unknown masternode'
  if (/not valid or PoSe banned/.test(text)) return 'Masternode not valid or banned'
  if (/masternode not valid/.test(text)) return 'Masternode not enabled'
  if (/Proposal not found/.test(text)) return 'Proposal unknown to the node'
  if (/Finalized Budget .* not found/.test(text)) return 'Finalized budget unknown to the node'
  if (/^bad-[a-z]+:\s*$/.test(text)) return 'Proposal not known yet (node syncing)'
  return text
}

const total = (pred) => (data.value?.totals || []).filter(pred).reduce((s, t) => s + t.count, 0)
const accepted = computed(() => total((t) => t.reason.startsWith('accepted')))
const rejected = computed(() => total((t) => !t.reason.startsWith('accepted') && !t.reason.startsWith('proposal removed')))
const otherCount = computed(() => (data.value?.others || []).reduce((s, o) => s + o.count, 0))
/* How strongly a row ties to its collateral. Only 'signed' says the
   masternode's key made this vote; a replay is its old vote resent by anyone,
   a claim is a collateral named in a vote whose signature was never checked. */
function evidence(r) {
  if (r.mn.method === 'matched') return r.mn.signed ? 'signed' : 'claimed'
  if (r.mn.method === 'known') return r.reason.startsWith('older') ? 'replayed' : 'signed'
  return null
}
const traced = computed(() => (data.value?.rejections || []).filter((r) => evidence(r) === 'signed').length)
const windowStart = computed(() => (data.value ? data.value.as_of - data.value.hours * 3600 : 0))
const coverage = computed(() => {
  const d = data.value
  if (!d.coverage_from) return { v: 'None', s: 'no log lines yet' }
  const partial = d.coverage_from > windowStart.value
  const s = `${partial ? 'window starts before logging; ' : ''}watching since ${shortTime(d.coverage_from)}`
  if (d.gaps.length) return { v: `${d.gaps.length} gap${d.gaps.length > 1 ? 's' : ''}`, s }
  return { v: partial ? 'Partial' : 'Complete', s }
})

const txid = (outpoint) => outpoint.split('-')[0]
/* The masternode's address from listmasternodes; a collateral no longer
   listed (spent or dropped) has none, and says so rather than guessing. */
const AddrOf = (props) => {
  const a = data.value?.addresses?.[props.op]
  return a
    ? h(RouterLink, { to: `/address/${a}`, class: 'addr' }, () => a)
    : h('span', { class: 'dim' }, 'not in current masternode list')
}
AddrOf.props = ['op']
const vout = (outpoint) => outpoint.split('-')[1]

// "Oct 20 08:46": the window is at most 7 days, the year adds nothing.
const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const pad = (n) => String(n).padStart(2, '0')
function shortTime(ts) {
  const d = new Date(ts * 1000)
  return `${MON[d.getMonth()]} ${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function detail(r) {
  if (r.reason.startsWith('under 1 h') && r.detail != null) return `${formatCount(r.detail)} s after the stored vote`
  if (r.reason.startsWith('signed over') && r.detail != null) return `signed for ${shortTime(r.detail)}`
  return ''
}

/* Rejections grouped by the collateral they trace to: the page's real question
   is which masternodes keep sending votes the node refuses. */
const blank = (op) => ({ op, signed: 0, replayed: 0, claimed: 0, reasons: {}, last: 0 })
const byMasternode = computed(() => {
  const groups = new Map()
  for (const r of data.value?.rejections || []) {
    const ops = r.mn.outpoint ? [r.mn.outpoint] : r.mn.outpoints || []
    if (ops.length !== 1) continue
    const op = ops[0]
    const g = groups.get(op) || blank(op)
    const n = r.seen || 1
    g[evidence(r)] += n
    const k = reasonOf(r.reason).short
    g.reasons[k] = (g.reasons[k] || 0) + n
    g.last = Math.max(g.last, r.ts)
    groups.set(op, g)
  }
  for (const o of data.value?.others || []) {
    if (o.mn?.outpoints?.length !== 1) continue
    const op = o.mn.outpoints[0]
    const g = groups.get(op) || blank(op)
    // Signature failures and unknown or invalid masternodes: the node never
    // verified that this collateral's key sent them.
    g.claimed += o.count
    const k = plainOther(o.text)
    g.reasons[k] = (g.reasons[k] || 0) + o.count
    g.last = Math.max(g.last, o.last_ts)
    groups.set(op, g)
  }
  return [...groups.values()].sort((a, b) => b.signed - a.signed || b.replayed - a.replayed || b.claimed - a.claimed || b.last - a.last)
})

/* Continuous hourly axis: quiet hours are shown as zero, not skipped. */
const chartOption = computed(() => {
  const p = palette()
  const d = data.value
  if (!d) return baseOption(p)
  const end = d.as_of - (d.as_of % 3600)
  const start = end - (d.hours - 1) * 3600
  const byHour = new Map(d.hourly.map((h) => [h.hour, h.counts]))
  const slots = []
  for (let t = start; t <= end; t += 3600) slots.push(t)
  const reasons = Object.keys(REASONS)
  const tone = { hot: p.hot, neon: p.neon, amber: p.amber }
  const series = [
    { name: 'Accepted', color: p.cyan, get: (c) => c.accepted || 0 },
    ...reasons.map((r) => ({ name: REASONS[r].short, color: tone[REASONS[r].tone], get: (c) => c[r] || 0 })),
  ]
  return {
    ...baseOption(p),
    legend: { top: 0, right: 0, itemWidth: 10, itemHeight: 10, textStyle: { color: p.text, fontSize: 11 } },
    grid: { left: 36, right: 8, top: 28, bottom: 24 },
    tooltip: { ...baseOption(p).tooltip, trigger: 'axis', axisPointer: { type: 'shadow' } },
    xAxis: {
      type: 'category',
      data: slots,
      axisTick: { show: false },
      axisLine: { lineStyle: { color: p.grid } },
      axisLabel: {
        color: p.axis, fontSize: 10, interval: d.hours > 48 ? 23 : 5,
        formatter: (t) => {
          const dt = new Date(t * 1000)
          return dt.getHours() === 0 || d.hours > 48 ? `${MON[dt.getMonth()]} ${dt.getDate()}` : `${pad(dt.getHours())}:00`
        },
      },
    },
    yAxis: { type: 'value', minInterval: 1, axisLabel: { color: p.axis, fontSize: 10 }, splitLine: { lineStyle: { color: p.grid } } },
    series: series.map((s) => ({
      name: s.name, type: 'bar', stack: 'v', barMaxWidth: 10,
      itemStyle: { color: s.color },
      emphasis: { focus: 'series' },
      data: slots.map((t) => s.get(byHour.get(t) || {})),
    })),
  }
})
</script>

<template>
  <div class="votelog">
    <div class="vl-head">
      <div class="seg" role="group" aria-label="Time window">
        <button v-for="h in [24, 168]" :key="h" type="button" class="modetab" :class="{ on: hours === h }" :aria-pressed="hours === h" @click="setHours(h)">
          {{ h === 24 ? 'Last 24 h' : 'Last 7 days' }}
        </button>
      </div>
      <span class="vl-meta" v-if="data">
        Updated {{ shortTime(data.as_of) }}. Checked against {{ formatCount(data.collaterals_known) }} known collaterals.
      </span>
      <span v-if="loading" class="vl-meta">Loading…</span>
    </div>

    <div v-if="error" class="banner warn">
      {{ error }}. The vote log needs the rustyblox-govlog sidecar and <code>governance.govlog_db_path</code> set on this explorer.
    </div>

    <template v-else-if="data">
      <div class="statgrid cols-4">
        <Stat k="ACCEPTED"><template #v>{{ formatCount(accepted) }}</template><template #s>votes the node took in</template></Stat>
        <Stat k="REJECTED" accent><template #v>{{ formatCount(rejected + otherCount) }}</template><template #s>{{ formatCount(rejected) }} with a vote hash, {{ formatCount(otherCount) }} without</template></Stat>
        <Stat k="TRACED"><template #v>{{ traced }}<span class="unit">/ {{ data.rejections.length }}</span></template><template #s>rejected votes signed by a known masternode</template></Stat>
        <Stat k="COVERAGE"><template #v>{{ coverage.v }}</template><template #s>{{ coverage.s }}</template></Stat>
      </div>

      <HudPanel title="VOTES PER HOUR" id="accepted and rejected, by reason">
        <EChart :option="chartOption" height="200px" aria-label="Votes per hour, accepted and rejected by reason" />
        <p v-if="!data.hourly.length" class="quiet">The node logged no budget votes in this window. Outside voting campaigns, votes arrive a few per hour at most.</p>
      </HudPanel>
    </template>

      <template v-if="groups && (groups.groups.length || groups.removals_error)">
        <h2 class="section-title">Identical ballots</h2>
        <p class="quiet lead">
          Masternodes that cast the same choices on two or more proposals, each signing its choices with
          no more than {{ groups.batch_gap_secs }} s between consecutive votes, in groups of {{ groups.min_group }} or more. Only votes the node counts are included.
          A group is matching choices; the times show how closely they were signed. Nothing here links the collaterals to one owner.
        </p>
        <p v-if="groups.removals_error" class="quiet lead">Node removals could not be read: {{ groups.removals_error }}</p>
        <HudPanel v-for="(g, i) in groups.groups" :key="i" :title="`${formatCount(g.size)} MASTERNODES`" :id="share(g.size)">
          <p class="group-line">
            Cast the same {{ g.choices_total ?? g.choices.length }} votes, signed between {{ shortTime(g.first) }} and {{ shortTime(g.last) }}
            ({{ span(g.last - g.first) }}): {{ formatCount(g.changed) }} changed, {{ formatCount(g.new) }} new.
            <template v-if="g.unknown_from">{{ formatCount(g.unknown_from) }} changed votes have no recorded prior choice and are left out of the net change.</template>
          </p>
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Proposal</th><th>Voted</th><th class="num" title="Change in yeas minus nays from this group's votes">Net change</th><th>Afterwards</th></tr></thead>
              <tbody>
                <tr v-for="c in g.choices" :key="c.proposal">
                  <td>
                    <RouterLink v-if="c.name" :to="`/proposal/${encodeURIComponent(c.name)}`">{{ c.name }}</RouterLink>
                    <span v-else class="mono dim">{{ truncateHash(c.proposal) }}</span>
                  </td>
                  <td><span class="tag" :class="c.choice === 1 ? 'cyan' : c.choice === 2 ? 'hot' : ''">{{ CHOICE[c.choice] }}</span></td>
                  <td class="num mono" :class="c.swing < 0 ? 'neg' : c.swing > 0 ? 'pos' : 'dim'">{{ signed(c.swing) }}</td>
                  <td>
                    <span v-if="c.removed" class="tag hot">Removed by the node: {{ c.removed.reason }}, {{ shortTime(c.removed.ts) }}</span>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <details class="members">
            <summary>{{ g.members_total > g.members.length ? `First ${g.members.length} of ${formatCount(g.members_total)} collaterals` : `${formatCount(g.members_total)} collaterals` }}</summary>
            <ul>
              <li v-for="m in g.members" :key="m.outpoint" class="mono">
                <RouterLink :to="`/tx/${txid(m.outpoint)}`">{{ truncateHash(txid(m.outpoint)) }}</RouterLink><span class="dim">:{{ vout(m.outpoint) }}</span>
                <RouterLink v-if="m.addr" :to="`/address/${m.addr}`" class="addr">{{ m.addr }}</RouterLink>
              </li>
            </ul>
          </details>
        </HudPanel>
      </template>

    <template v-if="data && !error">

      <template v-if="byMasternode.length">
        <h2 class="section-title">Masternodes with rejected votes</h2>
        <HudPanel title="BY MASTERNODE" :id="`${byMasternode.length} collaterals`">
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Collateral</th><th>Rejected for</th><th class="num" title="Refused after the node verified this masternode's signature">Signed</th><th class="num" title="This masternode's earlier votes, resent by an unknown peer">Replayed</th><th class="num" title="Votes naming this collateral whose signature was never verified">Claimed</th><th class="num">Last</th></tr></thead>
              <tbody>
                <tr v-for="g in byMasternode" :key="g.op">
                  <td class="mono">
                    <RouterLink :to="`/tx/${txid(g.op)}`">{{ truncateHash(txid(g.op)) }}</RouterLink><span class="dim">:{{ vout(g.op) }}</span>
                    <div class="sub"><AddrOf :op="g.op" /></div>
                  </td>
                  <td><span v-for="(n, k) in g.reasons" :key="k" class="reason">{{ k }} <b>{{ n }}</b></span></td>
                  <td class="num mono" :class="g.signed ? 'strong' : 'dim'">{{ formatCount(g.signed) }}</td>
                  <td class="num mono" :class="g.replayed ? '' : 'dim'">{{ formatCount(g.replayed) }}</td>
                  <td class="num mono" :class="g.claimed ? '' : 'dim'">{{ formatCount(g.claimed) }}</td>
                  <td class="num mono dim">{{ shortTime(g.last) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </HudPanel>
      </template>

      <h2 class="section-title">Rejected votes</h2>
      <HudPanel title="REJECTED" :id="`latest ${data.rejections.length}`">
        <dl class="key" aria-label="Evidence, strongest first">
          <div v-for="(m, k) in METHODS" :key="k" class="key-item"><dt><span class="tag" :class="m.tone">{{ m.label }}</span></dt><dd>{{ m.what }}</dd></div>
        </dl>
        <div class="scroll" v-if="data.rejections.length">
          <table class="dtable">
            <thead><tr><th>Time</th><th>Rejected as</th><th>Proposal</th><th>Masternode</th><th>Evidence</th></tr></thead>
            <tbody>
              <tr v-for="r in data.rejections" :key="r.vote + r.ts">
                <td class="mono dim" :title="formatDateTime(r.ts)">{{ shortTime(r.ts) }}</td>
                <td>
                  <span class="tag" :class="reasonOf(r.reason).tone">{{ reasonOf(r.reason).short }}</span>
                  <span v-if="r.seen > 1" class="mono dim"> ×{{ r.seen }}</span>
                  <div class="sub">{{ detail(r) }}</div>
                </td>
                <td>
                  <RouterLink v-if="r.object_name && r.object_kind === 'proposal'" :to="`/proposal/${encodeURIComponent(r.object_name)}`">{{ r.object_name }}</RouterLink>
                  <span v-else-if="r.object_name">{{ r.object_name }}</span>
                  <span v-else-if="r.object" class="mono dim">{{ truncateHash(r.object) }}</span>
                  <span v-else class="dim">Unknown</span>
                </td>
                <td class="mono">
                  <template v-if="r.mn.outpoint">
                    <RouterLink :to="`/tx/${txid(r.mn.outpoint)}`">{{ truncateHash(txid(r.mn.outpoint)) }}</RouterLink><span class="dim">:{{ vout(r.mn.outpoint) }}</span>
                    <div class="sub"><AddrOf :op="r.mn.outpoint" /></div>
                    <div v-if="r.mn.vote != null" class="sub">voted {{ CHOICE[r.mn.vote] }}</div>
                  </template>
                  <template v-else-if="r.mn.outpoints">
                    <div v-for="o in r.mn.outpoints" :key="o"><RouterLink :to="`/tx/${txid(o)}`">{{ truncateHash(txid(o)) }}</RouterLink><span class="dim">:{{ vout(o) }}</span></div>
                  </template>
                  <span v-else class="dim">{{ r.mn_logged || 'Not named' }}</span>
                </td>
                <td>
                  <span class="tag" :class="methodOf(r).tone">{{ methodOf(r).label }}</span>
                  <div v-if="r.mn.seen_at" class="sub">first seen {{ shortTime(r.mn.seen_at) }}</div>
                  <div v-else-if="r.mn.reason" class="sub">{{ r.mn.reason }}</div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-else class="quiet">No rejected votes in this window.</p>
      </HudPanel>

      <template v-if="data.others.length">
        <h2 class="section-title">Rejected without a vote hash</h2>
        <HudPanel title="OTHER REJECTIONS" :id="data.others_total > data.others.length ? `top ${data.others.length} of ${formatCount(data.others_total)}` : 'signature, unknown or invalid masternode'">
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Reason</th><th>Masternode</th><th class="num">Count</th><th class="num">Last</th></tr></thead>
              <tbody>
                <tr v-for="o in data.others" :key="o.class + o.text">
                  <td>{{ plainOther(o.text) }}<div class="sub mono">{{ o.text }}</div></td>
                  <td class="mono">
                    <template v-if="o.mn && o.mn.outpoints">
                      <div v-for="x in o.mn.outpoints" :key="x"><RouterLink :to="`/tx/${txid(x)}`">{{ truncateHash(txid(x)) }}</RouterLink><span class="dim">:{{ vout(x) }}</span><div class="sub"><AddrOf :op="x" /></div></div>
                      <div v-if="o.mn.outpoints.length > 1" class="sub">prefix fits {{ o.mn.outpoints.length }} collaterals</div>
                    </template>
                    <span v-else class="dim">{{ o.mn ? o.mn.reason : 'Not named in the log' }}</span>
                  </td>
                  <td class="num mono strong">{{ formatCount(o.count) }}</td>
                  <td class="num mono dim">{{ shortTime(o.last_ts) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </HudPanel>
      </template>

      <template v-if="data.removals.length">
        <h2 class="section-title">Proposals removed by the node</h2>
        <HudPanel title="REMOVED" id="dropped from the node's budget">
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Time</th><th>Proposal</th><th>Reason</th></tr></thead>
              <tbody>
                <tr v-for="x in data.removals" :key="x.proposal + x.ts">
                  <td class="mono dim">{{ shortTime(x.ts) }}</td>
                  <td>{{ x.name || truncateHash(x.proposal) }}</td>
                  <td>{{ x.reason }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </HudPanel>
      </template>

      <template v-if="data.gaps.length">
        <h2 class="section-title">Coverage gaps</h2>
        <HudPanel title="GAPS" id="log lines that may be missing">
          <ul class="gaps">
            <li v-for="g in data.gaps" :key="g.from + '-' + g.until"><span class="mono">{{ shortTime(g.from) }} to {{ shortTime(g.until) }}</span> {{ g.reason }}</li>
          </ul>
        </HudPanel>
      </template>
    </template>

  </div>
</template>

<style scoped>
.vl-head { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-3) var(--space-4); margin-bottom: var(--space-4); }
.seg { display: inline-flex; gap: 3px; padding: 3px;
  background: rgba(10,6,20,0.5); border: 1px solid var(--hud-line); border-radius: var(--radius-md); }
/* Mirrors Governance.vue .modetab: scoped styles there do not reach this component. */
.modetab { font-family: var(--font-mono); font-size: 11px; font-weight: 600; letter-spacing: 0.1em;
  padding: 5px 14px; border-radius: var(--radius-sm); cursor: pointer;
  background: transparent; border: 1px solid transparent; color: var(--text-dim); transition: all .15s; }
.modetab:hover { color: var(--text-muted); }
.modetab:focus-visible { outline: 1px solid var(--neon); outline-offset: 1px; }
.modetab.on { color: var(--neon); background: rgba(196,107,255,0.12); border-color: var(--glass-edge); text-shadow: var(--glow-xs); }
.vl-meta { color: var(--text-dim); font-size: 12px; }
.votelog > .statgrid { margin-bottom: var(--space-5); }
.quiet { margin: var(--space-3) 0 0; color: var(--text-dim); font-size: 12.5px; }
.sub { color: var(--text-dim); font-size: 11px; margin-top: 3px; }

/* One tag shape for reasons and evidence; the tone carries the meaning.
   rgba mirrors tokens.css --cyan/--rose/--amber/--hot/--neon. */
.tag {
  display: inline-block; padding: 1px 7px; border-radius: 3px; font-size: 11px; line-height: 1.6;
  border: 1px solid var(--hud-line); color: var(--text-muted); white-space: nowrap;
}
.tag.cyan { color: var(--cyan); border-color: rgba(70, 230, 208, 0.45); }
.tag.rose { color: var(--rose); border-color: rgba(255, 111, 156, 0.5); background: rgba(255, 111, 156, 0.1); }
.tag.amber { color: var(--amber); border-color: rgba(255, 207, 92, 0.45); }
.tag.neon { color: var(--neon); border-color: rgba(196, 107, 255, 0.5); }
.tag.hot { color: var(--hot); border-color: rgba(255, 84, 112, 0.5); }
td .tag + .tag { margin-left: 6px; }
.tag.unverified { border-style: dashed; border-color: rgba(255, 255, 255, 0.28); }
.tag.none { border-color: transparent; color: var(--text-dim); padding-left: 0; }

.reason { display: inline-block; margin-right: 12px; color: var(--text-muted); font-size: 12px; }
.reason b { color: var(--text); font-weight: 600; margin-left: 2px; }

.key { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 8px 28px;
  margin: 0 0 var(--space-4); font-size: 12px; color: var(--text-muted); }
/* Tag and first text line share one 20px line box, so they centre on each other. */
.key-item { display: grid; grid-template-columns: 104px 1fr; align-items: start; gap: 12px; }
.key dt, .key dd { margin: 0; }
.key dd { line-height: 20px; }
.key .tag { display: block; width: 100%; text-align: center; box-sizing: border-box;
  line-height: 18px; padding-top: 0; padding-bottom: 0; font-family: var(--font-mono); }
.key .tag.none { padding-left: 7px; border: 1px solid var(--hud-line); border-style: dotted; }

.lead { margin: 0 0 var(--space-3); max-width: 78ch; }
.group-line { margin: 0 0 var(--space-3); color: var(--text-muted); font-size: 12.5px; }
.num.neg { color: var(--hot); }
.num.pos { color: var(--cyan); }
.members { margin-top: var(--space-3); font-size: 12px; }
.members summary { cursor: pointer; color: var(--text-muted); }
.members ul { list-style: none; margin: var(--space-2) 0 0; padding: 0; columns: 2 420px; column-gap: 28px; }
.members li { padding: 3px 0; break-inside: avoid; }
.members .addr { margin-left: 10px; }
.panel + .panel { margin-top: var(--space-4); }
.gaps { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-muted); }
.gaps li + li { margin-top: 4px; }

@media (max-width: 640px) {
  .key { grid-template-columns: 1fr; }
}
</style>
