// Provisional static a11y/SEO lint for .brain markup (QA). Mirrors SPEC §8 codes; stand-in until `brain check`.
// Covers the markup-only rules: B0401 B0402 B0403 B0404 B0405 B0406 B0407 B0501. CSS rules (B0408 contrast,
// B0409 reduced-motion) need the token resolver → measured only through `brain check` and Lighthouse.
// Usage: node a11y.mjs <file.brain>...  → one JSON line per file: { file, diags: [{code,line,col,msg}] }
import { readFileSync } from 'node:fs'

const INTERACTIVE = new Set(['button', 'a', 'input', 'select', 'textarea', 'summary', 'details', 'label', 'option'])

export function lint(src, file = '<input>') {
  const diags = []
  const at = (i) => ({ line: src.slice(0, i).split('\n').length, col: i - src.lastIndexOf('\n', i - 1) })
  const add = (code, i, msg) => diags.push({ code, ...at(i), msg })
  // Blank out script/style so their contents never look like tags (keep offsets).
  const blank = (s) => s.replace(/[^\n]/g, ' ')
  const markup = src.replace(/<script>[\s\S]*?<\/script>|<style>[\s\S]*?<\/style>/g, blank)
  const tags = [...markup.matchAll(/<([a-zA-Z][\w-]*)((?:[^>"'{]|"[^"]*"|'[^']*'|\{[^}]*\})*)>/g)]
  const isView = /(^|\/)views\//.test(file)
  if (isView && !/<meta\s+seo\b/.test(src)) add('B0501', src.search(/<(?!meta)[a-zA-Z]/) >= 0 ? src.search(/<(?!meta)[a-zA-Z]/) : 0, 'view without <meta seo>')

  const forIds = new Set([...markup.matchAll(/<label\b[^>]*\bfor="([^"]+)"/g)].map((m) => m[1]))
  const labelled = (attrs) => /\baria-label(ledby)?=/.test(attrs) || forIds.has((attrs.match(/\bid="([^"]+)"/) || [])[1])
  let lastHeading = 0, h1 = 0, main = 0
  for (const m of tags) {
    const [, tag, attrs] = m, i = m.index, t = tag.toLowerCase()
    if (t === 'meta' || t === 'script' || t === 'style') continue
    if (t === 'img' && !/\balt=/.test(attrs)) add('B0401', i, '<img> without alt')
    if (/\bon:click=/.test(attrs) && !INTERACTIVE.has(t) && /^[a-z]/.test(tag) && !/\brole="(button|link)"/.test(attrs)) add('B0402', i, `on:click on non-interactive <${t}>`)
    if (['input', 'select', 'textarea'].includes(t) && !/type="(hidden|submit|button)"/.test(attrs) && !labelled(attrs)) {
      const before = markup.slice(0, i), open = before.lastIndexOf('<label'), close = before.lastIndexOf('</label>')
      if (!(open > close)) add('B0403', i, `<${t}> without <label> or aria-label`)
    }
    const h = t.match(/^h([1-6])$/)
    if (h) {
      const lv = Number(h[1])
      if (lastHeading && lv > lastHeading + 1) add('B0404', i, `heading skip h${lastHeading} → h${lv}`)
      lastHeading = lv
      if (lv === 1 && isView && ++h1 > 1) add('B0405', i, 'more than one <h1> in view')
    }
    if (t === 'main' && isView && ++main > 1) add('B0405', i, 'more than one <main> in view')
    if (t === 'button' && !/\baria-label=/.test(attrs)) {
      const end = markup.indexOf('</button>', i + m[0].length)
      const inner = markup.slice(i + m[0].length, end).replace(/<[^>]*>/g, '').trim()
      if (end >= 0 && inner === '') add('B0406', i, '<button> without accessible text')
    }
    const tab = attrs.match(/\btabindex="(\d+)"/)
    if (tab && Number(tab[1]) > 0) add('B0407', i + m[0].indexOf('tabindex'), 'tabindex > 0')
  }
  return { file, diags }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  for (const f of process.argv.slice(2)) console.log(JSON.stringify(lint(readFileSync(f, 'utf8'), f)))
}
