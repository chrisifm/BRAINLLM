// Provisional static checker for .brain files (QA, Phase 0). Stand-in until `brain check` exists.
// Checks: block order, forbidden BrainScript syntax, foreign-framework syntax, balanced blocks,
// keyed each, undeclared identifiers (script + markup), bind targets are sig.
// Usage: node check.mjs <file.brain>...   → prints one JSON line per file: { file, ok, errors[] }
import { readFileSync } from 'node:fs'

const KEYWORDS = new Set(('let const function return if else for of in await async try catch finally throw new ' +
  'typeof instanceof void delete true false null undefined import from export default as while do break continue ' +
  'switch case').split(' '))
const BASE_GLOBALS = new Set(('emit ref log load props ctx Math JSON Date Promise fetch Array Object String ' +
  'Number Boolean Error Map Set RegExp Intl console setTimeout clearTimeout setInterval clearInterval ' +
  'structuredClone encodeURIComponent decodeURIComponent parseInt parseFloat isNaN Infinity NaN URL ' +
  'URLSearchParams FormData crypto localStorage sessionStorage navigator requestAnimationFrame').split(' '))
const FOREIGN = [
  [/\buseState\b|\buseEffect\b|\buseMemo\b/, 'React hooks'],
  [/\bonMount\b|\$:\s|\bexport\s+let\b/, 'Svelte API'],
  [/\sv-(if|for|model|on|bind)\b|\s@click=|:key=/, 'Vue directives'],
  [/\sonClick=|\sclassName=/, 'JSX attributes'],
  [/\bcreateSignal\b|\bcreateEffect\b/, 'Solid API'],
]
const FORBIDDEN = [
  [/\bclass\s+[A-Za-z_$][\w$]*\s*(extends\b[^{]*)?\{/, 'B0101 class'],
  [/\bthis\b/, 'B0102 this'],
  [/\bvar\s/, 'B0103 var'],
  [/\bwith\s*\(/, 'B0104 with'],
  [/\beval\s*\(/, 'B0105 eval'],
  [/\bnew\s+Function\b/, 'B0106 new Function'],
  [/\bfunction\s*\*/, 'B0107 generator'],
  [/\b(get|set)\s+[A-Za-z_$][\w$]*\s*\(\s*[\w$]*\s*\)\s*\{/, 'B0108 getter/setter'],
  [/\bfor\s*\(\s*(let|const)?\s*[\w$]+\s+in\s/, 'B0109 for in'],
]

// Remove strings/comments (keeping template-literal ${} expressions) so regexes only see code.
function stripCode(src) {
  let out = '', i = 0
  while (i < src.length) {
    const c = src[i], n = src[i + 1]
    if (c === '/' && n === '/') { while (i < src.length && src[i] !== '\n') i++; continue }
    if (c === '/' && n === '*') { i = src.indexOf('*/', i + 2); i = i < 0 ? src.length : i + 2; out += ' '; continue }
    if (c === '"' || c === "'") {
      i++; while (i < src.length && src[i] !== c) { if (src[i] === '\\') i++; i++ } i++; out += '""'; continue
    }
    if (c === '`') {
      i++; out += '""'
      while (i < src.length && src[i] !== '`') {
        if (src[i] === '\\') { i += 2; continue }
        if (src[i] === '$' && src[i + 1] === '{') {
          let depth = 1, j = i + 2
          while (j < src.length && depth) { if (src[j] === '{') depth++; else if (src[j] === '}') depth--; j++ }
          out += ' ' + stripCode(src.slice(i + 2, j - 1)) + ' '; i = j; continue
        }
        i++
      }
      i++; continue
    }
    out += c; i++
  }
  return out
}

const IDENT = /[A-Za-z_$][\w$]*/g

// Names introduced by a code fragment (declarations, params, destructuring, catch, for-of).
function declaredIn(code) {
  const names = new Set()
  const addAll = (frag) => { for (const m of frag.matchAll(IDENT)) if (!KEYWORDS.has(m[0])) names.add(m[0]) }
  for (const m of code.matchAll(/\b(?:let|const|function)\s+([A-Za-z_$][\w$]*)/g)) names.add(m[1])
  for (const m of code.matchAll(/\b(?:let|const)\s*([{[][^=]*?[}\]])\s*=/g)) addAll(m[1].replace(/:\s*[A-Za-z_$][\w$]*/g, (s) => s))
  for (const m of code.matchAll(/\(([^()]*)\)\s*=>/g)) addAll(m[1].replace(/=[^,]*/g, ''))
  for (const m of code.matchAll(/([A-Za-z_$][\w$]*)\s*=>/g)) names.add(m[1])
  for (const m of code.matchAll(/\bfunction\s*[\w$]*\s*\(([^)]*)\)/g)) addAll(m[1].replace(/=[^,]*/g, ''))
  for (const m of code.matchAll(/\bcatch\s*\(\s*([\w$]+)/g)) names.add(m[1])
  for (const m of code.matchAll(/\bimport\s+([\w$]+)/g)) names.add(m[1])
  return names
}

// Identifiers read by a code fragment: not after `.`, not object keys (`k:` outside ternaries).
function usedIn(code) {
  const used = []
  for (const m of code.matchAll(IDENT)) {
    const name = m[0], start = m.index, end = start + name.length
    if (KEYWORDS.has(name)) continue
    const before = code.slice(0, start).trimEnd()
    if (before.endsWith('.') && !before.endsWith('...')) continue
    const after = code.slice(end).trimStart()
    if (after.startsWith(':') && !after.startsWith('::')) {
      const prev = before.slice(-1)
      if (prev === '{' || prev === ',') continue // object key
    }
    used.push(name)
  }
  return used
}

// api: framework primitive names (default SPEC names; Phase 0 variant `apishort` uses s/d/f).
export const API_DEFAULT = { sig: 'sig', drv: 'drv', fx: 'fx' }
export function check(src, file = '<input>', api = API_DEFAULT) {
  const GLOBALS = new Set([...BASE_GLOBALS, api.sig, api.drv, api.fx])
  const errors = []
  const err = (code, msg) => errors.push({ code, msg })
  const metaM = src.match(/^\s*((?:<meta\s[^>]*>\s*)*)/)
  const metas = metaM[1]
  const propsAttr = metas.match(/<meta\s+props="([^"]*)"/)
  const props = new Set()
  if (propsAttr) for (const p of propsAttr[1].trim().split(/\s+(?=[\w$]+:)/)) if (p) props.add(p.split(':')[0])

  const scripts = [...src.matchAll(/<script>([\s\S]*?)<\/script>/g)]
  const styles = [...src.matchAll(/<style>[\s\S]*?<\/style>/g)]
  if (scripts.length > 1) err('Q001', 'more than one <script>')
  if (styles.length > 1) err('Q001', 'more than one <style>')
  const idx = (re) => { const m = src.search(re); return m < 0 ? null : m }
  const iScript = idx(/<script>/), iStyle = idx(/<style>/)
  const body = src.slice(metas.length)
  const firstMarkup = body.replace(/<script>[\s\S]*?<\/script>/, '').replace(/<style>[\s\S]*?<\/style>/, '').search(/\S/)
  if (iScript !== null && iScript < metas.length) err('Q001', 'block order: <meta> must precede <script>')
  if (iScript !== null && body.trimStart().startsWith('<script>') === false && firstMarkup >= 0) err('Q001', 'block order: <script> must come right after <meta>')
  if (iStyle !== null && src.slice(src.indexOf('</style>') + 8).trim() !== '') err('Q001', 'block order: <style> must be last')

  const scriptRaw = scripts.map((m) => m[1]).join('\n')
  const script = stripCode(scriptRaw)
  for (const [re, what] of FORBIDDEN) if (re.test(script)) err(what.slice(0, 5), `forbidden: ${what.slice(6)}`)
  for (const [re, what] of FOREIGN) if (re.test(src)) err('Q002', `foreign syntax: ${what}`)

  const markup = src.slice(metas.length).replace(/<script>[\s\S]*?<\/script>/g, '').replace(/<style>[\s\S]*?<\/style>/g, '')
  for (const blk of ['if', 'each', 'await']) {
    const open = (markup.match(new RegExp(`\\{#${blk}\\b`, 'g')) || []).length
    const close = (markup.match(new RegExp(`\\{/${blk}\\}`, 'g')) || []).length
    if (open !== close) err('Q003', `unbalanced {#${blk}} (${open} open, ${close} close)`)
  }
  for (const m of markup.matchAll(/\{#each\s+([^}]*)\}/g)) {
    if (!/\bas\s+[\w$]+(\s*,\s*[\w$]+)?\s*\([^)]+\)\s*$/.test(m[1].trim())) err('Q004', `each without key: {#each ${m[1].trim()}}`)
  }

  // Markup expressions: {expr}, attr={expr}; control blocks contribute their aliases.
  const exprs = []
  const aliases = new Set()
  const re = /\{([^{}]*(?:\{[^{}]*\}[^{}]*)*)\}/g
  for (const m of markup.matchAll(re)) {
    let e = m[1].trim()
    if (e.startsWith('/') || e === ':else' || e === ':empty') continue
    let mm
    if ((mm = e.match(/^#each\s+([\s\S]+?)\s+as\s+([\w$]+)(?:\s*,\s*([\w$]+))?\s*(?:\(([\s\S]*)\))?$/))) {
      aliases.add(mm[2]); if (mm[3]) aliases.add(mm[3]); exprs.push(mm[1]); continue
    }
    if ((mm = e.match(/^:then\s+([\w$]+)/)) || (mm = e.match(/^:catch\s+([\w$]+)/))) { aliases.add(mm[1]); continue }
    e = e.replace(/^#(if|await)\s+/, '').replace(/^:else\s+if\s+/, '')
    exprs.push(e)
  }
  // each keys reference the alias; check them after aliases are known
  for (const m of markup.matchAll(/\{#each[^}]*\(([^)]*)\)\s*\}/g)) exprs.push(m[1])

  const declared = new Set([...declaredIn(script), ...props, ...GLOBALS])
  const sigs = new Set([...script.matchAll(new RegExp(`\\b(?:let|const)\\s+([\\w$]+)\\s*=\\s*${api.sig}\\(`, 'g'))].map((m) => m[1]))
  const refs = new Set([...script.matchAll(/\b(?:let|const)\s+([\w$]+)\s*=\s*ref\(/g)].map((m) => m[1]))
  const undeclared = new Set()
  for (const name of usedIn(script)) if (!declared.has(name)) undeclared.add(name)
  for (const e of exprs) {
    const code = stripCode(e)
    const local = new Set([...declaredIn(code), ...aliases])
    for (const name of usedIn(code)) if (!declared.has(name) && !local.has(name)) undeclared.add(name)
  }
  for (const name of undeclared) err('Q005', `undeclared identifier: ${name}`)

  for (const m of markup.matchAll(/\bbind:(value|checked|ref)=\{\s*([\w$.]+)\s*\}/g)) {
    const target = m[2]
    if (m[1] === 'ref' ? !refs.has(target) : !sigs.has(target)) err('Q006', `bind:${m[1]} target is not a ${m[1] === 'ref' ? 'ref()' : 'sig()'}: ${target}`)
  }
  for (const m of markup.matchAll(/\bbind:([\w-]+)=/g)) if (!['value', 'checked', 'ref'].includes(m[1])) err('Q007', `unknown bind:${m[1]}`)
  // Assignments to non-sig, non-let names from markup statements are fine (plain lets); assigning to a drv is not.
  const drvs = new Set([...script.matchAll(new RegExp(`\\b(?:let|const)\\s+([\\w$]+)\\s*=\\s*${api.drv}\\(`, 'g'))].map((m) => m[1]))
  for (const e of exprs) for (const d of drvs) if (new RegExp(`(^|[^.\\w$])${d.replace('$', '\\$')}\\s*(=[^=]|\\+\\+|--)`).test(e)) err('Q008', `assignment to drv: ${d}`)
  for (const d of drvs) if (new RegExp(`(^|[^.\\w$])${d.replace('$', '\\$')}\\s*(=[^=>]|\\+\\+|--)`).test(script.replace(new RegExp(`(let|const)\\s+${d.replace('$', '\\$')}\\s*=`), ''))) err('Q008', `assignment to drv: ${d}`)

  // Q009 (warning, not an error): a sig that is written but never read → state with no effect (incomplete logic).
  const warnings = []
  const allCode = script + '\n' + exprs.map(stripCode).join('\n')
  for (const name of sigs) {
    const esc = name.replace(/\$/g, '\\$')
    const reads = [...allCode.matchAll(new RegExp(`(^|[^.\\w$])${esc}(?![\\w$])(?!\\s*(=(?!=)|\\+\\+|--))`, 'g'))]
      .filter((m) => !/(let|const)\s+$/.test(allCode.slice(0, m.index + m[1].length)))
    if (reads.length === 0) warnings.push({ code: 'Q009', msg: `sig written but never read: ${name}` })
  }
  return { file, ok: errors.length === 0, errors, warnings }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  let bad = 0
  for (const f of process.argv.slice(2)) {
    const r = check(readFileSync(f, 'utf8'), f)
    if (!r.ok) bad++
    console.log(JSON.stringify(r))
  }
  process.exit(bad ? 1 : 0)
}
