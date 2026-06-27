/**
 * Safe CSV generation + download.
 *
 * Hardened against the three ways naive CSV export corrupts data:
 *  - fields containing commas / quotes / newlines (every field is quote-wrapped,
 *    internal quotes doubled per RFC 4180) -- proposal Names routinely contain commas;
 *  - non-ASCII text rendering as mojibake in Excel (a UTF-8 BOM is prepended);
 *  - CSV injection: a field starting with = + - @ (or a control char) is a formula
 *    vector in spreadsheets, so we prefix those with a single quote. Proposal Name/URL
 *    are attacker-controllable, so this matters.
 */

/** Quote-wrap one cell, escaping quotes and neutralizing formula triggers. */
function csvCell(value) {
  let s = value === null || value === undefined ? '' : String(value)
  // CSV-injection guard: neutralize spreadsheet formula triggers (= + - @ and control
  // chars) -- but NOT plain numbers, which legitimately start with '-' (e.g. negative
  // net votes). A bare numeric literal can't be a formula, so leave it intact.
  const isPlainNumber = /^-?\d+(\.\d+)?$/.test(s)
  if (/^[=+\-@\t\r]/.test(s) && !isPlainNumber) s = `'${s}`
  // RFC 4180: escape embedded double-quotes by doubling, then wrap the whole field.
  return `"${s.replace(/"/g, '""')}"`
}

/**
 * Build a CSV string.
 * @param {Array<{key: string, label: string}>} columns
 * @param {Array<Object>} rows - objects keyed by each column's `key`
 * @returns {string} CSV text with CRLF line endings (no BOM; downloadCsv adds it)
 */
export function toCsv(columns, rows) {
  const header = columns.map(c => csvCell(c.label)).join(',')
  const body = rows.map(row => columns.map(c => csvCell(row[c.key])).join(','))
  return [header, ...body].join('\r\n')
}

/**
 * Trigger a browser download of CSV text. Prepends a UTF-8 BOM so Excel decodes
 * non-ASCII characters correctly, and revokes the object URL afterwards.
 * @param {string} filename
 * @param {string} csv - output of toCsv()
 */
export function downloadCsv(filename, csv) {
  const BOM = String.fromCharCode(0xfeff)
  const blob = new Blob([BOM, csv], { type: 'text/csv;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
  URL.revokeObjectURL(url)
}
