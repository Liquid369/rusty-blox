# Legacy PIVX Explorer Frontend

⚠️ **DEPRECATED** - This is the legacy single-file Vue 3 explorer. All new development is in `/frontend-vue`.

## What This Is

The original PIVX block explorer - a self-contained single HTML file (~3,500 lines) with:
- Vue 3 (CDN-loaded)
- Axios (CDN-loaded)
- Inline CSS and JavaScript
- No build process

## Why It's Preserved

This frontend is kept as:
- **Historical reference** for existing functionality
- **Fallback option** if needed during migration
- **Example code** for porting features to the new Vue 3 app

## How to Run

### Option 1: Nginx (Recommended)

If running the Rust backend with nginx:
```bash
# The nginx.conf already serves this folder
# Just access: http://localhost:8080
```

### Option 2: Python HTTP Server

```bash
cd /Users/liquid/Projects/rusty-blox/frontend-legacy
python3 -m http.server 8000
# Access: http://localhost:8000
```

### Option 3: Node.js HTTP Server

```bash
cd /Users/liquid/Projects/rusty-blox/frontend-legacy
npx serve -p 8000
# Access: http://localhost:8000
```

## Configuration

The API endpoint is hardcoded in `index.html`:
```javascript
const API_BASE = window.location.origin;
```

Modify this if your backend is on a different port.

## Features Implemented

✅ Recent blocks list with pagination
✅ Block detail view with transaction list
✅ Transaction detail with inputs/outputs
✅ Address detail with balance and history
✅ Search (blocks, transactions, addresses)
✅ Dark/light theme toggle
✅ Live updates via polling
✅ Mobile responsive design

## Migration Status

🚧 **Being replaced by:** `/frontend-vue` (Vue 3 + Vite + proper architecture)

The new frontend follows:
- Official PIVX branding (Montserrat typography, official purple palette)
- Component-based architecture (Pinia state management, Vue Router)
- Enterprise-grade design patterns
- Full design blueprint in `/FRONTEND_DESIGN_BLUEPRINT.md`

## DO NOT MODIFY

All new features, bug fixes, and improvements go into `/frontend-vue`.

This folder is **READ-ONLY** for reference purposes only.

---

**Last Updated:** December 29, 2025  
**Status:** Legacy / Deprecated  
**Replacement:** `/frontend-vue`
