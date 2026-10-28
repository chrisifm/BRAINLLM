// #21 final — functional smoke of the two generated sites in headless Chrome (CDP, no deps) and the HMR target.
// Selectors are semantic (label text, button text, roles, aria-*), not tied to one generated app, so the same
// script runs on the Sonnet and the Haiku output. Every check records its evidence; nothing is inferred.
import { readFileSync, existsSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { openPage } from '../../lib/cdp.mjs'

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

// In-page helpers (installed on every evaluate).
export const HELPERS = `(() => { if (window.__s) return; const vis = (e) => !!e && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const txt = (e) => (e?.innerText ?? e?.textContent ?? '').replace(/\\s+/g, ' ').trim();
  const labelOf = (el) => { const l = []; if (el.id) document.querySelectorAll('label[for="' + el.id + '"]').forEach((x) => l.push(txt(x)));
    const w = el.closest('label'); if (w) l.push(txt(w)); if (el.getAttribute('aria-label')) l.push(el.getAttribute('aria-label'));
    const lb = el.getAttribute('aria-labelledby'); if (lb) lb.split(' ').forEach((id) => l.push(txt(document.getElementById(id)))); if (el.placeholder) l.push(el.placeholder); return l.join(' | ') };
  window.__s = { vis, txt, labelOf,
    btn: (re, root = document) => [...root.querySelectorAll('button, a[role=button], [role=button], input[type=submit], [role=switch], [role=tab], [role=radio]')].find((b) => vis(b) && re.test(txt(b) || b.value || b.getAttribute('aria-label') || '')),
    field: (re, root = document) => [...root.querySelectorAll('input, textarea, select')].find((f) => vis(f) && re.test(labelOf(f))),
    set: (el, v) => { const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : el.tagName === 'SELECT' ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true }));
      el.dispatchEvent(new FocusEvent('blur')); el.dispatchEvent(new FocusEvent('focusout', { bubbles: true })); return el.value },
    money: (s) => [...String(s).matchAll(/\\$\\s?([\\d,]+(?:\\.\\d+)?)/g)].map((m) => Number(m[1].replace(/,/g, ''))),
    has: (re) => re.test(txt(document.body)),
    pressed: (c) => c.hasAttribute('aria-pressed') ? c.getAttribute('aria-pressed') !== 'false' : c.hasAttribute('aria-checked') ? c.getAttribute('aria-checked') !== 'false' : 'checked' in c && c.type !== 'submit' && c.type !== 'button' && c.tagName === 'INPUT' ? c.checked : /\b(on|active|selected)\b/.test(c.className),
  } })(); `
const H = (expr) => `${HELPERS}(() => { const s = window.__s; ${expr} })()`

async function session(port, url) {
  const page = await openPage(port, 'about:blank')
  const errors = [], warnings = []
  page.on((m) => {
    if (m.method === 'Runtime.exceptionThrown') errors.push(`exception: ${m.params.exceptionDetails?.exception?.description ?? m.params.exceptionDetails?.text}`.slice(0, 300))
    if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') errors.push(`console.error: ${m.params.args.map((a) => a.value ?? a.description).join(' ')}`.slice(0, 300))
    if (m.method === 'Log.entryAdded' && m.params.entry.level === 'error') errors.push(`log: ${m.params.entry.text} ${m.params.entry.url ?? ''}`.slice(0, 300))
    if (m.method === 'Log.entryAdded' && m.params.entry.level === 'warning') warnings.push(m.params.entry.text.slice(0, 200))
  })
  await page.send('Log.enable')
  await page.send('Page.navigate', { url })
  await sleep(2000)
  const ev = (expr) => page.evaluate(H(expr))
  const until = async (expr, timeout = 4000) => { for (let t = 0; t < timeout; t += 50) { if (await ev(expr).catch(() => false)) return true; await sleep(50) } return false }
  return { page, errors, warnings, ev, until }
}

const check = (out, name, pass, evidence) => { out.checks.push({ name, pass: !!pass, evidence }); return pass }

async function landing(port, base) {
  const out = { checks: [] }
  const { page, errors, warnings, ev, until } = await session(port, `${base}/`)
  try {
    // Pricing: the container that holds the three plan names; prices before and after the yearly control.
    const pricing = `const root = [...document.querySelectorAll('section, [id*=pric]')].filter((e) => /Basic/.test(s.txt(e)) && /Enterprise/.test(s.txt(e))).sort((a, b) => s.txt(a).length - s.txt(b).length)[0];`
    const monthly = await ev(`${pricing} return root ? s.money(s.txt(root)) : null`)
    const toggled = await ev(`${pricing} const c = s.btn(/year|annual/i, root) ?? [...root.querySelectorAll('input')].find((i) => /year|annual/i.test(s.labelOf(i)));
      if (!c) return null; c.click(); window.__yc = c; return c.tagName + ':' + (s.txt(c) || s.labelOf(c))`)
    await sleep(300)
    // Re-query the control: a prerendered island resumes on its first event and replaces its nodes, so the element
    // clicked above may be detached by now (the old reference would still show the pre-click attributes).
    const aria = await ev(`${pricing} const c = window.__yc?.isConnected ? window.__yc : (s.btn(/year|annual/i, root) ?? [...root.querySelectorAll('input')].find((i) => /year|annual/i.test(s.labelOf(i))));
      return c ? { pressedAttr: c.getAttribute('aria-pressed'), checkedAttr: c.getAttribute('aria-checked'), state: s.pressed(c), reQueried: !window.__yc?.isConnected } : null`)
    if (aria && (aria.pressedAttr !== null || aria.checkedAttr !== null)) check(out, 'pricing: toggle exposes aria-pressed/aria-checked="true" when on', aria.pressedAttr === 'true' || aria.checkedAttr === 'true', aria)
    const yearly = await ev(`${pricing} return root ? s.money(s.txt(root)) : null`)
    const pairs = (monthly ?? []).map((m, i) => [m, yearly?.[i]]).filter(([m]) => m > 0)
    const ratios = pairs.map(([m, y]) => +(y / m).toFixed(4))
    // 20 % off: per month (×0.80) or per year (×12×0.80 = ×9.6); rounding to whole units tolerated (±0.5).
    const ok = pairs.length >= 3 && pairs.every(([m, y]) => Math.abs(y - m * 0.8) <= 0.5 || Math.abs(y - m * 9.6) <= 0.5)
    check(out, 'pricing toggle: yearly = monthly −20 %', ok, { control: toggled, monthly, yearly, ratios })
    // Pro highlighted: the Pro card differs from the Basic card in border/background/shadow, or says "popular/recommended".
    const pro = await ev(`${pricing} const card = (re) => { const h = [...root.querySelectorAll('h2, h3, h4, strong')].find((e) => re.test(s.txt(e))); return h ? h.closest('li, article, [class*=plan], [class*=card]') : null };
      const st = (e) => { if (!e) return null; const c = getComputedStyle(e); return [c.borderTopWidth, c.borderTopColor, c.backgroundColor, c.boxShadow, c.transform].join(' | ') };
      const p = card(/^Pro(?![a-z])/), b = card(/^Basic\\b/);
      return { pro: st(p), basic: st(b), proClass: p?.className ?? null, proText: p ? s.txt(p).slice(0, 40) : null, badge: p ? /popular|recommended|best/i.test(s.txt(p)) : null }`)
    check(out, 'pricing: Pro plan highlighted', !!pro.pro && (pro.pro !== pro.basic || pro.badge), pro)

    // FAQ: the smallest block titled FAQ / "frequently" with ≥ 6 buttons (or <details>). Open = the block's visible text
    // grows after the first click and shrinks back after the second. The aria-expanded values are kept as evidence.
    const fq = `const root = [...document.querySelectorAll('section, div, [id*=faq]')].filter((e) => /frequently|faq/i.test(s.txt(e).slice(0, 80)) && [...e.querySelectorAll('button')].filter(s.vis).length >= 6).sort((a, b) => s.txt(a).length - s.txt(b).length)[0];
      const bs = root ? [...root.querySelectorAll('button')].filter(s.vis) : [];`
    const faq = await ev(`${fq} return { buttons: bs.length, details: document.querySelectorAll('details').length }`)
    let faqOk = false, faqEv = { ...faq }
    if (faq.buttons >= 6) {
      const step = async () => ev(`${fq} const b = bs[0]; const id = b.getAttribute('aria-controls'); const target = id ? document.getElementById(id) : null;
        return { len: s.txt(root).length, ariaExpanded: b.getAttribute('aria-expanded'), ariaControls: id, controlsTag: target ? target.tagName + '#' + target.id : null }`)
      const s0 = await step()
      await ev(`${fq} bs[0].click(); return true`); await sleep(200)
      const s1 = await step()
      await ev(`${fq} bs[0].click(); return true`); await sleep(200)
      const s2 = await step()
      faqEv = { ...faq, before: s0, open: s1, closed: s2 }
      faqOk = s1.len > s0.len && s2.len === s0.len
      check(out, 'FAQ: aria-expanded is "false" when collapsed and "true" when open', s0.ariaExpanded === 'false' && s1.ariaExpanded === 'true', { collapsed: s0.ariaExpanded, open: s1.ariaExpanded })
      check(out, 'FAQ: aria-controls points to the answer, not to another element', s1.controlsTag !== null && !/^SECTION/.test(s1.controlsTag), { ariaControls: s1.ariaControls, resolvesTo: s1.controlsTag })
    } else if (faq.details >= 6) {
      const st = () => ev(`const d = document.querySelectorAll('details')[0]; return d.open`)
      const s0 = await st(); await ev(`document.querySelectorAll('details summary')[0].click(); return true`); await sleep(200)
      const s1 = await st(); await ev(`document.querySelectorAll('details summary')[0].click(); return true`); await sleep(200)
      const s2 = await st(); faqEv = { ...faq, s0, s1, s2 }; faqOk = !s0 && s1 && !s2
    }
    check(out, 'FAQ: 6 items expand and collapse', faqOk, faqEv)

    // Contact form: live validation and submit.
    const form = `const n = s.field(/name/i), e = s.field(/e-?mail/i), m = s.field(/message/i); const send = s.btn(/send/i);`
    const f0 = await ev(`${form} return { found: !!(n && e && m && send), disabled: send?.disabled ?? null }`)
    check(out, 'contact: Send disabled while empty', f0.found && f0.disabled === true, f0)
    await ev(`${form} s.set(n, 'Ana'); s.set(e, 'ana-at-example'); s.set(m, 'short'); return true`); await sleep(200)
    const f1 = await ev(`${form} return { disabled: send.disabled, invalid: [n, e, m].map((x) => x.getAttribute('aria-invalid')), errors: [...new Set([...document.querySelectorAll('[role=alert], .err, .error, [id$=err], [id$=error]'),
      ...[n, e, typeof m === "undefined" ? null : m].filter(Boolean).flatMap((x) => ((x.getAttribute('aria-describedby') ?? '') + ' ' + (x.getAttribute('aria-errormessage') ?? '')).split(/\\s+/).filter(Boolean).map((id) => document.getElementById(id)))].filter(Boolean))].map(s.txt).filter(Boolean) }`)
    check(out, 'contact: live validation flags email without @ and message < 10 chars', f1.disabled === true && f1.errors.some((x) => /@|email/i.test(x)) && f1.errors.some((x) => /10|message/i.test(x)), f1)
    await ev(`${form} s.set(e, 'ana@example.com'); s.set(m, 'I would like a demo, please.'); return true`); await sleep(200)
    const f2 = await ev(`${form} return { disabled: send.disabled }`)
    check(out, 'contact: Send enabled when valid', f2.disabled === false, f2)
    await ev(`${form} send.click(); return true`)
    const thanks = await until(`return s.has(/Thanks, we.ll be in touch soon/)`)
    check(out, 'contact: submit shows the thank-you message', thanks, { url: await page.evaluate('location.pathname') })
  } catch (e) { check(out, 'smoke ran to the end', false, e.message) }
  out.consoleErrors = errors
  out.consoleWarnings = [...new Set(warnings)]
  check(out, 'no console errors', errors.length === 0, errors.slice(0, 5))
  check(out, 'no SRI integrity mismatch warnings', !warnings.some((w) => /integrity/i.test(w)), warnings.filter((w) => /integrity/i.test(w)).slice(0, 3))
  page.close()
  return out
}

async function panel(port, base, route) {
  const out = { checks: [] }
  const { page, errors, warnings, ev, until } = await session(port, `${base}${route}`)
  const rows = `return [...document.querySelectorAll('tbody tr')].filter(s.vis).map((r) => [...r.querySelectorAll('td')].map(s.txt))`
  const kpi = `const k = (re) => { const el = [...document.querySelectorAll('body *')].filter((e) => e.children.length === 0 && re.test(s.txt(e)))[0];
      if (!el) return null; let c = el.parentElement; for (let i = 0; i < 3 && c && !/\\d/.test(s.txt(c).replace(s.txt(el), '')); i++) c = c.parentElement;
      const v = s.txt(c).replace(s.txt(el), ''); const m = v.match(/-?\\$?\\s?[\\d,]+(?:\\.\\d+)?/); return m ? Number(m[0].replace(/[$,\\s]/g, '')) : null };
    return { total: k(/total customers/i), active: k(/active customers/i), billed: k(/billed/i), ticket: k(/average ticket|avg/i) }`
  try {
    const loaded = await until(`return document.querySelectorAll('tbody tr').length > 0`, 6000)
    const main0 = await ev(`return s.txt(document.querySelector('main')).slice(0, 200)`)
    check(out, 'panel: customers load without any user interaction (8 s)', loaded, loaded ? null : { after8s: main0 })
    if (!loaded) { // an island only hydrates on the first event inside it: an input event on the search box wakes it
      await ev(`const q = [...document.querySelectorAll('input')].find((i) => s.vis(i) && !i.closest('form')); if (q) s.set(q, ''); return !!q`)
      check(out, 'panel: customers load after the first interaction', await until(`return document.querySelectorAll('tbody tr').length > 0`, 6000), null)
    }
    const all = await ev(rows)
    const k0 = await ev(kpi)
    const statusCol = all[0] ? all[0].findIndex((c) => /^(active|inactive)$/i.test(c)) : -1
    const active = statusCol >= 0 ? all.filter((r) => /^active$/i.test(r[statusCol])).length : null
    check(out, 'KPIs derived: total = rows, active = active rows, average = billed / active (±0.5 display rounding)', k0.total === all.length && k0.active === active &&
      (k0.active ? Math.abs(k0.ticket - k0.billed / k0.active) <= 0.5 : true), { kpi: k0, rows: all.length, activeRows: active })

    // Filters: the search box and the plan select outside any form.
    const filt = `const q = [...document.querySelectorAll('input')].find((i) => s.vis(i) && !i.closest('form') && /search|filter|name|email/i.test(s.labelOf(i) + ' ' + i.type));
      const p = [...document.querySelectorAll('select')].find((x) => s.vis(x) && !x.closest('form') && /plan/i.test(s.labelOf(x)));`
    const name = all[0]?.[0] ?? '', email = all[1]?.[1] ?? ''
    const byName = name.split(' ')[0].toLowerCase(), byEmail = email.split('@')[1] ?? email
    await ev(`${filt} s.set(q, ${JSON.stringify(byName)}); return true`); await sleep(250)
    const r1 = await ev(rows)
    await ev(`${filt} s.set(q, ${JSON.stringify(byEmail)}); return true`); await sleep(250)
    const r2 = await ev(rows)
    check(out, 'filter by name / email', r1.length >= 1 && r1.every((r) => r.join(' ').toLowerCase().includes(byName)) && r2.length >= 1 && r2.every((r) => r.join(' ').toLowerCase().includes(byEmail.toLowerCase())),
      { name: byName, nameRows: r1.length, email: byEmail, emailRows: r2.length })
    await ev(`${filt} s.set(q, 'zzqqxx-none'); return true`); await sleep(250)
    const noRes = await ev(`return { rows: document.querySelectorAll('tbody tr').length, text: s.has(/no (results|customers match|matches)/i) }`)
    check(out, 'empty state for "no results for the filter"', noRes.text && noRes.rows === 0, noRes)
    await ev(`${filt} s.set(q, ''); return true`); await sleep(250)
    // An option's value may be an id ('starter') while the table shows its label ('Starter'): match a row cell by either.
    const plans = await ev(`${filt} return p ? [...p.options].map((o) => ({ value: o.value, text: s.txt(o) })) : null`)
    const opt = (plans ?? []).find((o) => o.value && !/all/i.test(o.value) && all.some((r) => r.includes(o.value) || r.includes(o.text)))
    const plan = opt ? (all.some((r) => r.includes(opt.value)) ? opt.value : opt.text) : undefined
    await ev(`${filt} s.set(p, ${JSON.stringify(opt?.value ?? '')}); return true`); await sleep(250)
    const r3 = await ev(rows)
    check(out, 'filter by plan', !!plan && r3.length >= 1 && r3.every((r) => r.includes(plan)) && r3.length === all.filter((r) => r.includes(plan)).length, { plan, value: opt?.value, rows: r3.length, expected: all.filter((r) => r.includes(plan)).length })
    await ev(`${filt} s.set(p, p.options[0].value); return true`); await sleep(250)

    // Create with validation.
    await ev(`s.btn(/new customer/i).click(); return true`); await sleep(300)
    const frm = `const f = [...document.querySelectorAll('form')].find(s.vis) ?? document; const n = s.field(/name/i, f), e = s.field(/e-?mail/i, f), pl = s.field(/plan/i, f); const save = s.btn(/save|create|add/i, f);`
    const shown = await ev(`${frm} return !!(n && e && save)`)
    check(out, 'create: form opens', shown, null)
    await ev(`${frm} s.set(n, ''); s.set(e, 'nobody'); save.click(); return true`); await sleep(400)
    const v1 = await ev(`${frm} return { rows: document.querySelectorAll('tbody tr').length, errors: [...new Set([...document.querySelectorAll('[role=alert], .err, .error, [id$=err], [id$=error]'),
      ...[n, e, typeof m === "undefined" ? null : m].filter(Boolean).flatMap((x) => ((x.getAttribute('aria-describedby') ?? '') + ' ' + (x.getAttribute('aria-errormessage') ?? '')).split(/\\s+/).filter(Boolean).map((id) => document.getElementById(id)))].filter(Boolean))].map(s.txt).filter(Boolean) }`)
    check(out, 'create: validation blocks empty name and email without @', v1.rows === all.length && v1.errors.some((x) => /name/i.test(x)) && v1.errors.some((x) => /@|email/i.test(x)), v1)
    await ev(`${frm} s.set(n, 'QA Smoke'); s.set(e, 'qa@smoke.test'); if (pl) s.set(pl, pl.options[pl.options.length - 1].value); save.click(); return true`)
    await sleep(40)
    const saving = await ev(`const b = [...document.querySelectorAll('button')].find((x) => /saving/i.test(s.txt(x))); return b ? { text: s.txt(b), disabled: b.disabled } : null`)
    check(out, 'create: button disabled and labeled "Saving…" while saving', saving?.disabled === true, saving)
    const created = await until(`return [...document.querySelectorAll('tbody tr')].some((r) => /QA Smoke/.test(s.txt(r)))`, 5000)
    const k1 = await ev(kpi)
    check(out, 'create: row added and KPIs update', created && k1.total === all.length + 1, { kpi: k1 })

    // Edit.
    await ev(`const r = [...document.querySelectorAll('tbody tr')].find((x) => /QA Smoke/.test(s.txt(x))); s.btn(/edit/i, r).click(); return true`); await sleep(300)
    const pre = await ev(`${frm} return n?.value`)
    await ev(`${frm} s.set(n, 'QA Smoke Edited'); save.click(); return true`)
    const edited = await until(`return [...document.querySelectorAll('tbody tr')].some((r) => /QA Smoke Edited/.test(s.txt(r)))`, 5000)
    check(out, 'edit: form prefilled and row updated', pre === 'QA Smoke' && edited, { prefilled: pre })

    // Delete.
    await ev(`const r = [...document.querySelectorAll('tbody tr')].find((x) => /QA Smoke Edited/.test(s.txt(x))); s.btn(/delete|remove/i, r).click(); return true`)
    await sleep(100)
    await ev(`const c = s.btn(/^(confirm|yes|delete)$/i, [...document.querySelectorAll('dialog, [role=dialog], [role=alertdialog]')].find(s.vis) ?? document.createElement('i')); if (c) c.click(); return true`)
    const deleted = await until(`return ![...document.querySelectorAll('tbody tr')].some((r) => /QA Smoke/.test(s.txt(r)))`, 5000)
    const k2 = await ev(kpi)
    check(out, 'delete: row removed and KPIs back', deleted && k2.total === all.length, { kpi: k2 })
  } catch (e) { check(out, 'smoke ran to the end', false, e.message) }
  out.consoleErrors = errors
  out.consoleWarnings = [...new Set(warnings)]
  check(out, 'no console errors', errors.length === 0, errors.slice(0, 5))
  check(out, 'no SRI integrity mismatch warnings', !warnings.some((w) => /integrity/i.test(w)), warnings.filter((w) => /integrity/i.test(w)).slice(0, 3))
  page.close()
  return out
}

export async function smoke(site, port, base, routes) {
  const r = site === 'landing' ? await landing(port, base) : await panel(port, base, routes.includes('/customers') ? '/customers' : '/')
  r.passed = r.checks.filter((c) => c.pass).length; r.total = r.checks.length
  return r
}

// HMR target: one component with both <style> and <script>; style edit = alternate the first spacing token in <style>,
// script edit = alternate the last string literal in <script> (changes the emitted JS). State is set in the page first.
export function hmrTarget(site, app) {
  const ls = (d) => existsSync(join(app, d)) ? readdirSync(join(app, d)).filter((f) => f.endsWith('.brain') && !f.includes('.test.')).map((f) => `${d}/${f}`) : []
  const both = (f) => { const s = readFileSync(join(app, f), 'utf8'); return /<style>[\s\S]*var\(--space-\d\)[\s\S]*<\/style>/.test(s) && /<script>[\s\S]*'[^'\n]{4,}'[\s\S]*<\/script>/.test(s) }
  const pref = site === 'landing' ? /pric/ : /form/
  const cands = [...ls('comp').filter((f) => pref.test(f)), ...ls('comp'), ...ls('views').filter((f) => /home|index|customers/.test(f))].filter(both)
  const file = cands[0]
  if (!file) return null
  const style = (src, i) => src.replace(/<style>[\s\S]*?<\/style>/, (blk) => blk.replace(/var\(--space-(\d)\)/, (m, n) => i % 2 ? m : `var(--space-${n === '1' ? 2 : Number(n) - 1})`))
  const script = (src, i) => src.replace(/<script>[\s\S]*?<\/script>/, (blk) => { const all = [...blk.matchAll(/'((?:[^'\\\n]|\\.)*)'/g)].filter((x) => x[1].length >= 4); const last = all.at(-1); if (!last || i % 2) return blk
    return blk.slice(0, last.index) + `'${last[1]}.'` + blk.slice(last.index + last[0].length) })
  if (site === 'landing') return { file, route: '/', style, script, expected: 'true',
    setState: H(`const c = s.btn(/year|annual/i) ?? [...document.querySelectorAll('input')].find((i) => /year|annual/i.test(s.labelOf(i))); if (!c) return 'no-control'; c.click();
      return new Promise((r) => setTimeout(() => r(String(s.pressed(c))), 200))`),
    readState: H(`const c = s.btn(/year|annual/i) ?? [...document.querySelectorAll('input')].find((i) => /year|annual/i.test(s.labelOf(i))); return c ? String(s.pressed(c)) : 'no-control'`) }
  return { file, route: '/', style, script, expected: 'HMR state',
    setState: H(`s.btn(/new customer/i)?.click(); return new Promise((r) => setTimeout(() => { const f = [...document.querySelectorAll('form')].find(s.vis); const n = f && s.field(/name/i, f); r(n ? s.set(n, 'HMR state') : 'no-form') }, 400))`),
    readState: H(`const f = [...document.querySelectorAll('form')].find(s.vis); const n = f && s.field(/name/i, f); return n ? n.value : 'no-form'`) }
}
