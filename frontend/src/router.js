import { createRouter, createWebHashHistory } from 'vue-router'

// Hash history keeps base './' deep-links working when a built prototype
// is served from an arbitrary sub-path (no server rewrite needed).
const routes = [
  { path: '/', name: 'dashboard', component: () => import('./views/Dashboard.vue') },
  { path: '/blocks', name: 'blocks', component: () => import('./views/BlockList.vue') },
  { path: '/block/:height', name: 'block', component: () => import('./views/BlockDetail.vue'), props: true },
  { path: '/tx/:txid', name: 'tx', component: () => import('./views/TransactionDetail.vue'), props: true },
  { path: '/address/:addr', name: 'address', component: () => import('./views/AddressDetail.vue'), props: true },
  { path: '/xpub/:xpub', name: 'xpub', component: () => import('./views/XPubDetail.vue'), props: true },
  { path: '/mempool', name: 'mempool', component: () => import('./views/Mempool.vue') },
  { path: '/masternodes', name: 'masternodes', component: () => import('./views/MasternodeList.vue') },
  { path: '/masternode/:id', name: 'masternode', component: () => import('./views/MasternodeDetail.vue'), props: true },
  { path: '/governance', name: 'governance', component: () => import('./views/Governance.vue') },
  { path: '/proposal/:name', name: 'proposal', component: () => import('./views/ProposalDetail.vue'), props: true },
  { path: '/analytics', name: 'analytics', component: () => import('./views/Analytics.vue') },
  { path: '/search/:query', name: 'search', component: () => import('./views/SearchResults.vue'), props: true },
  { path: '/:pathMatch(.*)*', name: 'notfound', component: () => import('./views/NotFound.vue') }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
  scrollBehavior() { return { top: 0 } }
})

// Per-page <title> so tabs, bookmarks, and history are meaningful (an SPA otherwise
// keeps the static index.html title on every route). Truncate long hashes for the tab.
const BRAND = 'RUSTY//BLOX'
const short = (s, n = 14) => (s && s.length > n ? s.slice(0, n) + '…' : s || '')
router.afterEach((to) => {
  const p = to.params
  const label = {
    dashboard: 'PIVX Mission Control',
    blocks: 'Blocks',
    block: `Block #${p.height}`,
    tx: `Transaction ${short(p.txid)}`,
    address: `Address ${short(p.addr)}`,
    xpub: `XPub ${short(p.xpub)}`,
    mempool: 'Mempool',
    masternodes: 'Masternodes',
    masternode: `Masternode ${short(p.id)}`,
    governance: 'Governance',
    proposal: `Proposal · ${decodeURIComponent(p.name || '')}`,
    analytics: 'Analytics',
    search: `Search · ${decodeURIComponent(p.query || '')}`,
    notfound: 'Not found',
  }[to.name] || 'PIVX Mission Control'
  document.title = `${label} — ${BRAND}`
})

export default router
