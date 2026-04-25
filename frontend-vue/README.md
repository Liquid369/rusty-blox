# PIVX Explorer - Vue 3 Frontend

Modern, enterprise-grade blockchain explorer frontend for PIVX built with Vue 3, Vite, and official PIVX branding.

## 🎨 Design System

**Official PIVX Branding:**
- **Colors:** Official purple palette from [PIVX Brand Identity Guide](https://pivx.org/pressroom)
  - Primary: `#662D91` (CMYK: 75 98 1 0)
  - Secondary: `#4D3077`, Tertiary: `#2A1B42`, Deep: `#130D1E`
  - Accent: `#59FCB3` (teal)
- **Typography:** Montserrat (200, 400, 700, 800 weights)
- **Architecture:** Follows `/FRONTEND_DESIGN_BLUEPRINT.md`

## 📦 Tech Stack

- **Framework:** Vue 3 (Composition API)
- **Build Tool:** Vite
- **State Management:** Pinia
- **Routing:** Vue Router 4
- **HTTP Client:** Axios
- **Charts:** Apache ECharts (future)
- **Styling:** CSS Variables + Utility Classes

## 🚀 Quick Start

### Install Dependencies

```bash
cd frontend-vue
npm install
```

### Development Server

```bash
npm run dev
```

Runs on `http://localhost:3000` with API proxy to `localhost:3001` (Rust backend).

### Production Build

```bash
npm run build
```

Outputs to `dist/` directory.

### Preview Production Build

```bash
npm run preview
```

## 🏗️ Project Structure

```
frontend-vue/
├── public/
│   └── PIVX-Horz-White.svg          # PIVX logo
├── src/
│   ├── assets/
│   │   └── styles/
│   │       ├── variables.css         # Design tokens (PIVX colors, typography)
│   │       ├── base.css              # Reset & typography
│   │       └── utilities.css         # Helper classes
│   ├── components/
│   │   ├── common/                   # Reusable UI components
│   │   │   ├── UiButton.vue
│   │   │   ├── UiCard.vue
│   │   │   └── StatCard.vue
│   │   └── layout/                   # Layout components
│   │       ├── AppLayout.vue
│   │       ├── AppHeader.vue
│   │       ├── AppFooter.vue
│   │       └── SearchBar.vue
│   ├── views/                        # Page components
│   │   ├── Dashboard.vue             # ✅ Implemented
│   │   ├── BlockList.vue             # ✅ Implemented
│   │   ├── BlockDetail.vue           # ✅ Implemented
│   │   ├── TransactionDetail.vue     # 🚧 Placeholder
│   │   ├── AddressDetail.vue         # 🚧 Placeholder
│   │   └── [... other views ...]     # 🚧 Placeholders
│   ├── stores/                       # Pinia stores
│   │   ├── chainStore.js             # Chain state (height, supply, etc.)
│   │   └── settingsStore.js          # User preferences (theme, pagination)
│   ├── services/                     # API layer
│   │   ├── api.js                    # Axios instance
│   │   └── index.js                  # Service exports (blockService, etc.)
│   ├── router/
│   │   └── index.js                  # Vue Router config
│   ├── App.vue                       # Root component
│   └── main.js                       # Entry point
├── package.json
├── vite.config.js
└── index.html
```

## 🎯 Implementation Status

### ✅ Completed (Phase A Foundation)

- [x] Project scaffolding (Vite + Vue 3)
- [x] Design system implementation (official PIVX colors, Montserrat typography)
- [x] Layout components (AppLayout, AppHeader, AppFooter, SearchBar)
- [x] Shared UI components (UiButton, UiCard, StatCard)
- [x] Pinia stores (chainStore, settingsStore)
- [x] API service layer (blockService, transactionService, etc.)
- [x] Vue Router with 14 routes
- [x] Dashboard view (live stats, recent blocks/txs)
- [x] Block List view (paginated)
- [x] Block Detail view
- [x] Theme toggle (dark/light mode)

### 🚧 In Progress (Next Steps)

- [ ] Transaction Detail view (full implementation)
- [ ] Address Detail view (balance, history, UTXOs)
- [ ] Mempool Dashboard
- [ ] Masternode List & Detail
- [ ] Governance Dashboard
- [ ] Analytics Dashboard with ECharts
- [ ] Search functionality
- [ ] WebSocket integration for live updates
- [ ] Mobile responsive optimization

## 🔌 API Integration

### Backend Connection

The frontend expects the Rust backend running on `http://localhost:3001`.

Vite proxy configuration (in `vite.config.js`):
```javascript
proxy: {
  '/api': {
    target: 'http://localhost:3001',
    changeOrigin: true
  }
}
```

### Environment Variables

Create `.env` file (optional):
```bash
VITE_API_BASE=http://localhost:3001/api
```

### API Endpoints Used

- `GET /api/status` - Chain info (height, supply, masternodes)
- `GET /api/blocks?limit=N&offset=N` - Recent blocks
- `GET /api/block/:id` - Block detail
- `GET /api/txs?limit=N&offset=N` - Recent transactions
- `GET /api/tx/:txid` - Transaction detail
- `GET /api/address/:address` - Address detail

## 🎨 Theming

### Dark Mode (Default)

Official PIVX purple palette with dark backgrounds.

### Light Mode

Toggle via sun/moon button in header. Preference saved to `localStorage`.

### CSS Variables

All colors, spacing, and typography defined in `/src/assets/styles/variables.css`.

Example:
```css
--pivx-purple-primary: #662D91;
--text-accent: #59FCB3;
--space-6: 1.5rem;
```

## 📱 Responsive Design

- **Desktop:** 1200px max-width container
- **Tablet:** 768px breakpoint
- **Mobile:** Optimized for touch targets (44px minimum)

## 🧪 Development Workflow

### 1. Start Rust Backend

```bash
cd /Users/liquid/Projects/rusty-blox
cargo run --release
```

Backend runs on `http://localhost:3001`.

### 2. Start Vue Frontend

```bash
cd frontend-vue
npm run dev
```

Frontend runs on `http://localhost:3000` with API proxy.

### 3. Test in Browser

Open `http://localhost:3000` in your browser.

## 🚢 Deployment

### Build for Production

```bash
npm run build
```

### Serve with Nginx

Update `nginx.conf` to serve `frontend-vue/dist`:

```nginx
location / {
    root /path/to/rusty-blox/frontend-vue/dist;
    try_files $uri $uri/ /index.html;
}
```

### Serve with Backend

The Rust backend can serve the static files from `frontend-vue/dist`.

## 🔄 Migration & Coexistence

### Legacy Frontend

The old single-file Vue app is preserved in `/frontend-legacy`.

**Access legacy:**
```bash
# If using nginx (assuming it still points to legacy)
http://localhost:8080

# Or serve directly
cd frontend-legacy
python3 -m http.server 8000
```

### New Frontend

The new Vue 3 app in `/frontend-vue` is the **active development target**.

**Parallel development:**
- Legacy: `http://localhost:8080` (nginx or manual server)
- New Vue: `http://localhost:3000` (Vite dev server)

### Deployment Switch

When ready to deploy new frontend:

1. Build production bundle: `npm run build`
2. Update nginx to serve `frontend-vue/dist`
3. Restart nginx
4. Legacy frontend remains in `/frontend-legacy` for reference

## 📋 Next Steps (Implementation Roadmap)

Following the design blueprint (`/FRONTEND_DESIGN_BLUEPRINT.md`), the implementation proceeds in phases:

### Phase B: Core Explorer (Weeks 3-4)
- [ ] Complete Transaction Detail page
- [ ] Complete Address Detail page (balance, history, UTXOs)
- [ ] Implement Search Results page
- [ ] Add copy-to-clipboard functionality
- [ ] Add QR code generation for addresses

### Phase C: PIVX Features (Weeks 5-6)
- [ ] Mempool Dashboard with live updates
- [ ] Fee Estimator
- [ ] Masternode List with filtering
- [ ] Masternode Detail page
- [ ] Governance Dashboard
- [ ] Proposal Detail page

### Phase D: Analytics (Weeks 7-8)
- [ ] Integrate Apache ECharts
- [ ] Supply & Distribution charts
- [ ] Transaction Volume charts
- [ ] Staking Analytics
- [ ] Network Health metrics
- [ ] Rich List

### Phase E: Polish (Weeks 9-10)
- [ ] Mobile optimization
- [ ] Performance tuning (lazy loading, code splitting)
- [ ] Accessibility audit (WCAG AA)
- [ ] Unit tests
- [ ] E2E tests
- [ ] Documentation

## 🤝 Contributing

When adding new features:

1. Follow the design blueprint specifications
2. Use official PIVX colors (`variables.css`)
3. Maintain component structure
4. Keep Pinia stores for shared state
5. Use API services (don't call Axios directly in components)
6. Add proper TypeScript types (if migrating to TS)

## 📚 Resources

- **Design Blueprint:** `/FRONTEND_DESIGN_BLUEPRINT.md`
- **PIVX Branding:** https://pivx.org/pressroom
- **Legacy Frontend:** `/frontend-legacy` (reference only)
- **Backend API:** Running Rust backend at `localhost:3001`

## 🐛 Troubleshooting

### API Errors

If API calls fail, ensure the Rust backend is running:
```bash
cd /Users/liquid/Projects/rusty-blox
cargo run --release
```

### Port Conflicts

If `localhost:3000` is in use:
```bash
# Edit vite.config.js and change server.port
# Or kill existing process
lsof -ti:3000 | xargs kill -9
```

### Missing Dependencies

```bash
rm -rf node_modules package-lock.json
npm install
```

---

**Status:** ✅ Phase A Complete | 🚧 Phase B In Progress  
**Last Updated:** December 29, 2025  
**Design Blueprint:** See `/FRONTEND_DESIGN_BLUEPRINT.md` for full specifications
