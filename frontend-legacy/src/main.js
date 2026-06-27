// Self-hosted tabular monospace (bundled + served from 'self', so it loads under
// the app's `font-src 'self'` CSP — unlike a CDN font). Used for hashes/amounts so
// digits column-align identically across OSes. Falls back to the system mono stack
// in --font-mono if these ever fail, so worst case is today's behavior.
import '@fontsource/jetbrains-mono/400.css'
import '@fontsource/jetbrains-mono/500.css'
import '@fontsource/jetbrains-mono/600.css'
import '@fontsource/jetbrains-mono/700.css'

import './assets/styles/variables.css'
import './assets/styles/base.css'
import './assets/styles/utilities.css'

import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'

const app = createApp(App)

app.use(createPinia())
app.use(router)

app.mount('#app')
