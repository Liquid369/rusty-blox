// ECharts renders to <canvas> and cannot read CSS custom properties at runtime.
// These constants mirror the brand tokens from src/assets/styles/variables.css
// so chart option objects get the same values as the rest of the UI.
// IMPORTANT: keep in sync with variables.css whenever brand tokens change.

export const CHART = {
  purple: '#662D91',          // OFFICIAL PIVX brand primary (per brand guide)
  purpleAccent: '#B359FC',    // --purple-accent
  purpleGradient: ['#8B3FE0', '#B359FC'],
  green: '#B3FF78',           // --green-accent
  greenDark: '#71BB3A',
  greenGradient: ['#9BE85C', '#B3FF78'],
  warning: '#f6ff78',         // --warning
  danger: '#EF4444',          // --danger
  textPrimary: '#FFFFFF',     // --text-primary
  textSecondary: '#B8B0C4',   // --text-secondary
  axisText: '#B2ABBE',        // --text-tertiary (note: updated value; the old #9B93A8 is stale)
  surfaceDeep: '#110B1B',     // --bg-primary
  tooltipBorder: '#662D91',   // OFFICIAL PIVX brand primary
  gridLine: 'rgba(102, 45, 145, 0.25)',  // #662D91 @ 25%
}
