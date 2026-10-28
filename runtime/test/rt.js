// Single import point for the runtime under test: BRAIN_RUNTIME=<path> tests another build (e.g. the minified one).
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const { BRAIN_RUNTIME } = process['env']
const url = BRAIN_RUNTIME ? pathToFileURL(resolve(BRAIN_RUNTIME)).href : new URL('../brain.js', import.meta.url).href
export const { sig, drv, fx, batch, untrack, tpl, insert, attr, cls, on, list, when, mount, hydrate } = await import(url)
// Sibling modules (brain-dev.js, brain-wc.js) load next to the tested build: they import ./brain.js relatively.
export const sibling = name => import(new URL('./' + name, url).href)
export const base = new URL('./', url).href // folder of the tested build (compiled fixtures: __RT__/)
