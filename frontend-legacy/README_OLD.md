# PIVX Blockchain Explorer - Frontend

A modern, feature-rich single-page blockchain explorer for PIVX built with Vue.js 3.

## 📁 Files

```
frontend/
├── index.html          # Self-contained explorer (all CSS/JS inline)
├── PIVX-Horz-White.svg # PIVX logo
└── README.md           # This file
```

**Design Philosophy:** Single self-contained HTML file for maximum portability and simplicity. All styles and JavaScript are embedded inline - no build process, no dependencies, no external files.

## Features

### 🎨 Modern UI/UX
- **Dark/Light Theme Toggle** - Switch between dark and light themes with persistent preference
- **Responsive Design** - Works seamlessly on desktop, tablet, and mobile devices
- **Smooth Animations** - Polished transitions and loading states
- **Custom Scrollbars** - Styled to match the PIVX theme

### 📦 Block Explorer
- **Recent Blocks View** - Browse the latest blocks with pagination
- **Block Details** - Complete block information including:
  - Block height, hash, and confirmations
  - Timestamp and size
  - Difficulty and nonce
  - Merkle root and previous/next block navigation
  - All transactions in the block

### 💸 Transaction Explorer
- **Transaction Details** - View complete transaction information:
  - Inputs and outputs with addresses and amounts
  - Transaction type (CoinBase, CoinStake, Regular, Sapling)
  - Fees and confirmations
  - Block inclusion details

### 🔍 Advanced Search
- **Universal Search** - Search by:
  - Block height or hash
  - Transaction ID
  - Address
- **Search History** - Quick access to recent searches
- **Smart Detection** - Automatically identifies search type

### 📍 Address Explorer
- **Address Information**:
  - Current balance
  - Total transaction count
  - Recent transactions list
  - Unspent outputs (UTXOs) with confirmations
- **UTXO Management** - View all spendable outputs for an address

### 🔄 Mempool Monitor
- **Real-time Mempool Stats**:
  - Pending transaction count
  - Mempool size in bytes
  - Total fees
- **Recent Mempool Transactions** - View pending transactions

### 🔐 Masternode Dashboard
- **Masternode Statistics**:
  - Total, enabled, and stable counts
  - IPv4, IPv6, and Onion node counts
- **Masternode List** with filtering:
  - Status (Enabled, Pre-Enabled, Expired)
  - Type filtering
  - Rank, address, and last seen information

### 📊 Network Statistics
- **Money Supply**:
  - Total PIV supply
  - Transparent vs. Shield supply breakdown
  - Shield percentage
- **Budget Proposals**:
  - Active proposals with voting stats
  - Payment information and schedules
  - Proposal URLs and details

### 📈 Interactive Charts
- **Block Difficulty Chart** - Visualize difficulty over last 50 blocks
- **Transaction Volume Chart** - Bar chart of transaction counts per block
- **Block Size Chart** - Track block sizes over time
- **Auto-refresh** - Charts update every 30 seconds

### ⚡ Live Updates
- **WebSocket Integration** - Real-time updates for:
  - New blocks
  - New transactions
  - Mempool changes
- **Auto-refresh** - Chain state updates every 10 seconds

## File Structure

```
frontend/
├── index.html          # Self-contained single-page explorer
├── PIVX-Horz-White.svg # PIVX logo asset
└── README.md           # Documentation
```

**Architecture:** Single-file design for portability
- All CSS embedded in `<style>` tags
- All JavaScript embedded inline
- External dependencies: Vue.js 3, Axios (loaded from CDN)
- No build process required
- Production-ready as-is

## Getting Started

### Prerequisites
- Rusty-Blox backend running on `http://localhost:3005`
- Modern web browser with JavaScript enabled

### Usage

1. **Start the backend:**
   ```bash
   cargo run --release
   ```

2. **Open the frontend:**
   ```bash
   open frontend/index.html
   ```
   Or serve via HTTP server:
   ```bash
   cd frontend && python3 -m http.server 8080
   # Visit http://localhost:8080
   ```

### Configuration

Edit `API_BASE` constant in index.html (around line 2300) to change backend URL:
```javascript
const API_BASE = 'http://localhost:3005';
```
- **WebSocket** - Real-time updates

### Customization

#### Theme Colors
Edit CSS variables in `styles.css`:
```css
:root {
    --primary: #6c2eb9;      /* Primary brand color */
    --secondary: #9d4edd;    /* Secondary color */
    --success: #10b981;      /* Success/positive */
    --danger: #ef4444;       /* Error/negative */
    --warning: #f59e0b;      /* Warning/pending */
}
```

#### API Endpoint
Change the API URL in `app.js`:
```javascript
data() {
    return {
        apiUrl: 'http://your-api-url:port',
        // ...
    }
}
```

## API Endpoints Used

The frontend consumes these backend API endpoints:

- `GET /api/v2/status` - Chain synchronization status
- `GET /api/v2/block-detail/{height}` - Block details
- `GET /api/v2/block-stats/{count}` - Block statistics for charts
- `GET /api/v2/tx/{txid}` - Transaction details
- `GET /api/v2/address/{address}` - Address information
- `GET /api/v2/utxo/{address}` - Address UTXOs
- `GET /api/v2/search/{query}` - Universal search
- `GET /api/v2/mempool` - Mempool information
- `GET /api/v2/mncount` - Masternode count
- `GET /api/v2/mnlist` - Masternode list
- `GET /api/v2/moneysupply` - Money supply stats
- `GET /api/v2/budgetinfo` - Budget proposals
- `WS /ws/blocks` - WebSocket for new blocks
- `WS /ws/transactions` - WebSocket for new transactions

## Browser Support

- Chrome 90+
- Firefox 88+
- Safari 14+
- Edge 90+

## Performance

- **Lazy Loading** - Charts only render when viewed
- **Pagination** - Large data sets are paginated
- **Debouncing** - Search and API calls are optimized
- **Caching** - Browser caches static resources
- **WebSocket** - Efficient real-time updates vs. polling

## Security

- **Read-only** - Frontend has no write capabilities
- **CORS Enabled** - Backend must allow cross-origin requests
- **Client-side Only** - No sensitive data storage
- **HTTPS Ready** - Works over secure connections

## Troubleshooting

### API Connection Issues
1. Verify backend is running: `curl http://localhost:3005/api/v2/status`
2. Check CORS settings in backend
3. Verify port 3005 is not blocked by firewall

### Charts Not Displaying
1. Ensure Chart.js CDN is accessible
2. Check browser console for errors
3. Verify `/api/v2/block-stats/{count}` endpoint works

### WebSocket Not Connecting
1. Check if backend WebSocket endpoints are enabled
2. Verify WebSocket port is accessible
3. Check browser console for connection errors

## Future Enhancements

- [ ] Rich list (top addresses by balance)
- [ ] Network graphs and visualizations
- [ ] Export data to CSV/JSON
- [ ] Transaction broadcasting interface
- [ ] Multi-language support
- [ ] PWA (Progressive Web App) support
- [ ] Advanced filtering and sorting
- [ ] Bookmark favorite addresses/blocks
- [ ] Notification preferences

## License

Same as the main rusty-blox project.

## Contributing

Contributions are welcome! Please ensure:
- Code follows existing style
- Features are tested in multiple browsers
- Documentation is updated
- No breaking changes to existing features
