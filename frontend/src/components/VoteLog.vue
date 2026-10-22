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
  recovered: { label: 'Recovered', tone: 'cyan', what: 'The node’s own copy of the rejected vote, read back from it: masternode, proposal, choice and signed time are exact.' },
  known: { label: 'Seen before', tone: 'rose', what: 'These exact vote fields are already in this explorer’s history. The hash excludes the signature, so this cannot tell a resend from an identical re-signing.' },
  claimed: { label: 'Claimed only', tone: 'unverified', what: 'The vote names this collateral, but its signature was never checked.' },
  prefix: { label: 'Prefix match', tone: 'unverified', what: 'Only the first 10 characters of the collateral were logged.' },
  not_identifiable: { label: 'Not traced', tone: 'none', what: 'No known collateral rebuilds this hash.' },
}
const methodOf = (r) => {
  if (r.mn.method === 'matched' && !r.mn.signed) return METHODS.claimed
  const m = METHODS[r.mn.method] || METHODS.not_identifiable
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
/* Rejections without a vote hash, one row per reason and collateral funding
   transaction (a prefix that fits several collaterals stays on its own row). */
const otherGroups = computed(() => {
  const m = new Map()
  for (const o of data.value?.others || []) {
    const ops = o.mn?.outpoints || []
    const tx = ops.length === 1 ? txid(ops[0]) : null
    const reason = plainOther(o.text)
    const key = tx ? `${reason}|${tx}` : `${reason}|${o.text}`
    const g = m.get(key) || { key, reason, txid: tx, vouts: [], texts: [], count: 0, last: 0,
      note: ops.length > 1 ? `prefix fits ${ops.length} collaterals` : (o.mn ? o.mn.reason : 'Not named in the log') }
    if (tx && !g.vouts.includes(Number(vout(ops[0])))) g.vouts.push(Number(vout(ops[0])))
    g.texts.push(o.text)
    g.count += o.count
    g.last = Math.max(g.last, o.last_ts)
    m.set(key, g)
  }
  for (const g of m.values()) g.vouts.sort((a, b) => a - b)
  return [...m.values()].sort((a, b) => b.count - a.count || b.last - a.last)
})
const traced = computed(() => (data.value?.rejections || []).filter((r) => r.evidence).length)
/* Evidence comes from the server (api/govlog.rs evidence()); one definition. */
const EVIDENCE = {
  signed: { label: 'Signed', tone: 'cyan', what: 'Signature verified; signed at or after the stored vote, within an hour of it.' },
  seen: { label: 'Seen before', tone: 'rose', what: 'These exact vote fields are already in this explorer’s history. The hash excludes the signature, so this cannot tell a resend from an identical re-signing.' },
  older: { label: 'Older vote', tone: 'amber', what: 'Signature verified, but signed earlier than the stored vote: an old vote sent again, or a new one backdated. The data cannot tell which.' },
  claimed: { label: 'Claimed only', tone: 'unverified', what: 'Names the collateral; the signature was never checked.' },
}
const evidenceOf = (e) => EVIDENCE[e] || { label: 'Not traced', tone: 'none' }
const range = (a, b) => (a == null ? '' : a === b || shortTime(a) === shortTime(b) ? shortTime(a) : `${shortTime(a)} to ${shortTime(b)}`)
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

    <details class="about">
      <summary>About this page</summary>
      <p>Every masternode vote on a budget proposal reaches the network as a signed message. This page shows the ones this explorer’s node received: the votes it accepted, the votes it rejected and why, and proposals it removed. Voting itself happens on masternodes; nothing here changes a vote.</p>
      <p>Common rejections: <b>too soon</b>, a masternode may change its vote at most once an hour; <b>older than stored</b>, the node already holds a newer vote from that masternode; <b>future-dated</b>, signed more than an hour ahead. Old votes are re-sent across the network all the time, especially after a proposal returns, so most rejections are routine.</p>
      <p>Each rejection carries an evidence level that says how firmly it ties to a masternode, and rejections are grouped by the transaction that created each masternode’s collateral. Matching ballots or a shared funding transaction show what was observed, not who controls a masternode.</p>
      <p>The data comes from one node’s log, so it reflects what that node saw. Times are your local time.</p>
    </details>

    <div v-if="error" class="banner warn">
      {{ error }}. The vote log needs the rustyblox-govlog sidecar and <code>governance.govlog_db_path</code> set on this explorer.
    </div>

    <template v-else-if="data">
      <div class="statgrid cols-4">
        <Stat k="ACCEPTED"><template #v>{{ formatCount(accepted) }}</template><template #s>votes the node took in</template></Stat>
        <Stat k="REJECTED" accent><template #v>{{ formatCount(rejected + otherCount) }}</template><template #s>{{ formatCount(rejected) }} with a vote hash, {{ formatCount(otherCount) }} without</template></Stat>
        <Stat k="TRACED"><template #v>{{ formatCount(traced) }}<span class="unit"> of {{ formatCount(data.rejections.length) }}</span></template><template #s>latest rejected votes tied to a collateral</template></Stat>
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
            Collaterals from {{ formatCount(g.funding_txs) }} funding {{ g.funding_txs === 1 ? 'transaction' : 'transactions' }}.
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

      <template v-if="data.funding?.length">
        <h2 class="section-title">Collateral funding transactions</h2>
        <HudPanel title="BY FUNDING TRANSACTION" :id="data.funding_total > data.funding.length ? `top ${data.funding.length} of ${formatCount(data.funding_total)}` : `${formatCount(data.funding_total)} transactions`">
          <p class="quiet lead">Rejected votes grouped by the transaction that created each masternode's collateral. Collaterals from one transaction were funded together.</p>
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Funding transaction</th><th class="num">Collaterals</th><th>Rejections</th><th class="num">Last</th></tr></thead>
              <tbody>
                <tr v-for="f in data.funding" :key="f.txid">
                  <td class="mono funding-tx">
                    <RouterLink :to="`/tx/${f.txid}`">{{ truncateHash(f.txid) }}</RouterLink>
                    <details v-if="f.vouts_total > 1" class="vouts"><summary>{{ f.vouts_total }} outputs</summary><span class="dim">{{ f.vouts.map((v) => ':' + v).join(' ') }}<template v-if="f.vouts.length < f.vouts_total"> and {{ f.vouts_total - f.vouts.length }} more</template></span></details>
                    <span v-else class="dim">:{{ f.vouts[0] }}</span>
                  </td>
                  <td class="num mono strong">{{ formatCount(f.vouts_total) }}</td>
                  <td class="ev-counts">
                    <span v-for="(e, k) in EVIDENCE" v-show="f[k]" :key="k" class="tag" :class="e.tone" :title="e.what"><b>{{ formatCount(f[k]) }}</b> {{ e.label.toLowerCase() }}</span>
                    <span v-if="f.untraced" class="tag none"><b>{{ formatCount(f.untraced) }}</b> not traced</span>
                  </td>
                  <td class="num mono dim when">{{ shortTime(f.last) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </HudPanel>
      </template>

      <h2 class="section-title">Rejected votes</h2>
      <HudPanel title="REJECTED" :id="data.clusters_total > data.clusters.length ? `top ${data.clusters.length} of ${data.clusters_total} groups` : `${data.clusters_total} groups`">
        <dl class="key" aria-label="Evidence, strongest first">
          <div v-for="(m, k) in EVIDENCE" :key="k" class="key-item"><dt><span class="tag" :class="m.tone">{{ m.label }}</span></dt><dd>{{ m.what }}</dd></div>
          <div class="key-item"><dt><span class="tag none">Not traced</span></dt><dd>No collateral is known for the vote.</dd></div>
        </dl>
        <p class="quiet lead">Every rejection in the window, grouped by reason, proposal, choice and evidence. Open a group for its collaterals by funding transaction.</p>
        <div class="clusters">
          <details v-for="(c, i) in data.clusters" :key="i" class="cluster">
            <summary>
              <span class="tag" :class="reasonOf(c.reason).tone">{{ reasonOf(c.reason).short }}</span>
              <span class="c-prop">
                <RouterLink v-if="c.object_name" :to="`/proposal/${encodeURIComponent(c.object_name)}`" @click.stop>{{ c.object_name }}</RouterLink>
                <span v-else-if="c.object" class="mono dim">{{ truncateHash(c.object) }}</span>
                <span v-else class="dim">Proposal not logged</span>
                <span v-if="c.choice != null" class="dim"> · {{ CHOICE[c.choice] }}</span>
              </span>
              <span class="tag" :class="evidenceOf(c.evidence).tone">{{ evidenceOf(c.evidence).label }}</span>
              <span class="c-num mono"><b>{{ formatCount(c.count) }}</b><span class="c-sub">{{ c.masternodes ? `from ${formatCount(c.masternodes)} ${c.masternodes === 1 ? 'masternode' : 'masternodes'}` : `${c.count === 1 ? 'rejection' : 'rejections'}, masternode not known` }}</span></span>
              <span class="c-time dim">arrived {{ range(c.first, c.last) }}<template v-if="c.signed_from != null">; signed {{ range(c.signed_from, c.signed_to) }}</template></span>
            </summary>
            <ul v-if="c.funding.length" class="groups">
              <li v-for="g in c.funding" :key="g.txid" class="mono g-row">
                <RouterLink :to="`/tx/${g.txid}`">{{ truncateHash(g.txid) }}</RouterLink>
                <span class="g-count"><b>{{ g.vouts.length }}</b> {{ g.vouts.length > 1 ? 'collaterals' : 'collateral' }}</span>
                <span class="dim g-vouts">{{ g.vouts.map((v) => ':' + v).join(' ') }}</span>
              </li>
              <li v-if="c.funding_total > c.funding.length || c.vouts_total > c.funding.reduce((n, g) => n + g.vouts.length, 0)" class="dim">List cut to fit: {{ formatCount(c.vouts_total) }} collaterals from {{ formatCount(c.funding_total) }} funding transactions in total.</li>
            </ul>
            <p v-else class="quiet">No collateral is known for these rejections.</p>
          </details>
        </div>
        <details v-if="data.rejections.length" class="latest">
          <summary>Latest {{ data.rejections.length }} individual rejections</summary>
          <dl class="key" aria-label="How each masternode was found">
            <div v-for="(m, k) in METHODS" :key="k" class="key-item"><dt><span class="tag" :class="m.tone">{{ m.label }}</span></dt><dd>{{ m.what }}</dd></div>
          </dl>
        <div class="scroll">
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
                  <div v-else-if="r.mn.method === 'recovered'" class="sub">signed {{ shortTime(r.mn.n_time) }}<template v-if="!r.mn.canonical">, nonstandard vin</template></div>
                  <div v-else-if="r.mn.reason" class="sub">{{ r.mn.reason }}</div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        </details>
        <p v-if="!data.rejections.length" class="quiet">No rejected votes in this window.</p>
      </HudPanel>

      <template v-if="data.others.length">
        <h2 class="section-title">Rejected without a vote hash</h2>
        <HudPanel title="OTHER REJECTIONS" :id="data.others_total > data.others.length ? `top ${data.others.length} of ${formatCount(data.others_total)}` : 'signature, unknown or invalid masternode'">
          <div class="scroll">
            <table class="dtable">
              <thead><tr><th>Reason</th><th>Masternode</th><th class="num">Count</th><th class="num">Last</th></tr></thead>
              <tbody>
                <tr v-for="g in otherGroups" :key="g.key">
                  <td>{{ g.reason }}<div v-if="g.texts.length === 1" class="sub mono">{{ g.texts[0] }}</div></td>
                  <td class="mono">
                    <template v-if="g.txid">
                      <RouterLink :to="`/tx/${g.txid}`">{{ truncateHash(g.txid) }}</RouterLink>
                      <details v-if="g.vouts.length > 1" class="vouts"><summary>{{ g.vouts.length }} outputs</summary><span class="dim">{{ g.vouts.map((v) => ':' + v).join(' ') }}</span></details>
                      <span v-else class="dim">:{{ g.vouts[0] }}<div class="sub"><AddrOf :op="`${g.txid}-${g.vouts[0]}`" /></div></span>
                    </template>
                    <span v-else class="dim">{{ g.note }}</span>
                  </td>
                  <td class="num mono strong">{{ formatCount(g.count) }}</td>
                  <td class="num mono dim">{{ shortTime(g.last) }}</td>
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
.clusters { display: grid; gap: 6px; }
.cluster { border: 1px solid var(--hud-line); border-radius: var(--radius-sm); background: rgba(10, 6, 20, 0.35); }
.cluster > summary { display: grid; grid-template-columns: 130px minmax(160px, 1.4fr) 104px minmax(170px, 1fr) minmax(220px, 1.6fr);
  align-items: center; gap: 12px; padding: 9px 12px; cursor: pointer; list-style: none; font-size: 12.5px; }
.cluster > summary::-webkit-details-marker { display: none; }
.cluster[open] > summary { border-bottom: 1px solid var(--hud-line); }
.cluster > summary .tag { justify-self: start; }
.c-num { color: var(--text-muted); }
.c-num b { color: var(--text); }
.c-sub { margin-left: 0.4em; }
.c-time { font-size: 11.5px; }
.groups { list-style: none; margin: 0; padding: 8px 12px 10px; font-size: 11.5px; display: grid; gap: 6px; }
.g-row { display: grid; grid-template-columns: 170px 110px 1fr; gap: 12px; align-items: baseline; }
.g-count { color: var(--text-muted); white-space: nowrap; }
.g-count b { color: var(--text); }
.g-vouts { line-height: 1.6; word-spacing: 2px; }
@media (max-width: 700px) {
  .g-row { grid-template-columns: 1fr auto; }
  .g-vouts { grid-column: 1 / -1; }
}
.vouts summary, .latest > summary { cursor: pointer; color: var(--text-muted); font-size: 11.5px; }
.vouts span { display: block; max-width: 60ch; white-space: normal; font-size: 11px; }
.latest { margin-top: var(--space-4); }
.latest > summary { margin-bottom: var(--space-2); }
@media (max-width: 900px) {
  .cluster > summary { grid-template-columns: 1fr 1fr; }
}
.about { margin: 0 0 var(--space-4); max-width: 78ch; font-size: 12.5px; color: var(--text-muted); }
.about > summary { cursor: pointer; color: var(--text-dim); }
.about p { margin: var(--space-2) 0 0; line-height: 1.55; }
.about b { color: var(--text); font-weight: 600; }
.ev-counts { white-space: normal; }
.funding-tx a, .funding-tx > .dim { white-space: nowrap; }
td.when { white-space: nowrap; }
.ev-counts .tag { margin: 2px 6px 2px 0; }
.ev-counts .tag b { font-weight: 700; }
.gaps { margin: 0; padding-left: 18px; font-size: 12px; color: var(--text-muted); }
.gaps li + li { margin-top: 4px; }

@media (max-width: 640px) {
  .key { grid-template-columns: 1fr; }
}
</style>
