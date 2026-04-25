import { createRouter, createWebHistory } from 'vue-router'

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    {
      path: '/',
      name: 'Dashboard',
      component: () => import('@/views/Dashboard.vue'),
      meta: { title: 'PIVX Explorer' }
    },
    {
      path: '/blocks',
      name: 'BlockList',
      component: () => import('@/views/BlockList.vue'),
      meta: { title: 'Latest Blocks' }
    },
    {
      path: '/block/:id',
      name: 'BlockDetail',
      component: () => import('@/views/BlockDetail.vue'),
      meta: { title: 'Block Details' }
    },
    {
      path: '/tx/:txid',
      name: 'TransactionDetail',
      component: () => import('@/views/TransactionDetail.vue'),
      meta: { title: 'Transaction Details' }
    },
    {
      path: '/address/:address',
      name: 'AddressDetail',
      component: () => import('@/views/AddressDetail.vue'),
      meta: { title: 'Address Details' }
    },
    {
      path: '/xpub/:xpub',
      name: 'XPubDetail',
      component: () => import('@/views/XPubDetail.vue'),
      meta: { title: 'XPub Tracking' }
    },
    {
      path: '/mempool',
      name: 'MempoolDashboard',
      component: () => import('@/views/MempoolDashboard.vue'),
      meta: { title: 'Mempool' }
    },
    {
      path: '/masternodes',
      name: 'MasternodeList',
      component: () => import('@/views/MasternodeList.vue'),
      meta: { title: 'Masternodes' }
    },
    {
      path: '/masternode/:id',
      name: 'MasternodeDetail',
      component: () => import('@/views/MasternodeDetail.vue'),
      meta: { title: 'Masternode Details' }
    },
    {
      path: '/governance',
      name: 'GovernanceDashboard',
      component: () => import('@/views/GovernanceDashboard.vue'),
      meta: { title: 'Governance' }
    },
    {
      path: '/proposal/:name',
      name: 'ProposalDetail',
      component: () => import('@/views/ProposalDetail.vue'),
      meta: { title: 'Proposal Details' }
    },
    {
      path: '/analytics',
      name: 'AnalyticsDashboard',
      component: () => import('@/views/AnalyticsDashboard.vue'),
      meta: { title: 'Analytics' }
    },
    {
      path: '/search',
      name: 'SearchResults',
      component: () => import('@/views/SearchResults.vue'),
      meta: { title: 'Search Results' }
    },
    {
      path: '/:pathMatch(.*)*',
      name: 'NotFound',
      component: () => import('@/views/NotFound.vue'),
      meta: { title: '404 Not Found' }
    }
  ],
  scrollBehavior(to, from, savedPosition) {
    if (savedPosition) {
      return savedPosition
    } else {
      return { top: 0 }
    }
  }
})

// Global navigation guard for meta titles
router.beforeEach((to, from, next) => {
  document.title = to.meta.title 
    ? `${to.meta.title} | PIVX Explorer` 
    : 'PIVX Blockchain Explorer'
  next()
})

export default router
