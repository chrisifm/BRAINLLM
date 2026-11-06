// The compiler must retain fallback messages even when no successful component calls t() for their keys.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'
const binary = process.env.BRAIN_BIN ?? new URL('../../target/debug/brain', import.meta.url).pathname
function brain(args, cwd) {
  const result = spawnSync(binary, args, { cwd, encoding: 'utf8', timeout: 120000, maxBuffer: 8 * 1024 * 1024 })
  assert.equal(result.status, 0, result.stdout + result.stderr)
  return JSON.parse(result.stdout)
}
test('a compiled per-request failure uses the declared app locale, public messages and available safe stylesheet', { timeout: 180000 }, async () => {
  const directory = mkdtempSync(join(tmpdir(), 'brain-compiled-fallback-'))
  let server
  try {
    brain(['new', 'app', '--json'], directory)
    const app = join(directory, 'app'), dist = join(app, 'dist')
    rmSync(join(app, 'views/_error.brain'), { force: true }) // Exercise the generic fallback, not the scaffold's authored template.
    const config = JSON.parse(readFileSync(join(app, 'brain.json'), 'utf8'))
    config.docs = false
    config.seo = { ...config.seo, minScore: 0 }
    config.i18n = { default: 'fr', locales: ['fr'] }
    writeFileSync(join(app, 'brain.json'), JSON.stringify(config))
    mkdirSync(join(app, 'i18n'), { recursive: true })
    writeFileSync(join(app, 'i18n/fr.json'), JSON.stringify({ 'http.error.title': 'Localized fallback heading', 'http.error.message': 'Localized safe message', 'http.error.reference': 'Localized reference', 'http.error.home': 'Localized home', 'unrelated.private': 'private-dictionary-marker' }))
    mkdirSync(join(app, 'views/failure'), { recursive: true })
    writeFileSync(join(app, 'views/failure/[id].brain'), '<meta seo title="Example per-request page" description="An example per-request page used to verify a safe generic error document when its data load fails."><script>export async function load(){throw new Error("private-failure-marker")}</script><main><h1>Example per-request page</h1></main>')
    const firstBuild = brain(['build', '--site', 'https://example.test', '--json'], app)
    const manifest = JSON.parse(readFileSync(join(dist, '_server/manifest.json'), 'utf8'))
    assert.equal(manifest.errorFallback.defaultLocale, 'fr')
    assert.equal(manifest.errorFallback.locales.fr['http.error.title'], 'Localized fallback heading')
    assert.ok(!JSON.stringify(manifest.errorFallback).includes('private-dictionary-marker'))
    const { createServer } = await import(pathToFileURL(join(dist, 'server.mjs')).href)
    server = createServer({ dist, log() {} })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    const origin = `http://127.0.0.1:${server.address().port}`
    const response = await fetch(origin + '/failure/example')
    assert.equal(response.status, 500)
    const html = await response.text()
    assert.match(html, /<html lang="fr">/)
    assert.match(html, /500 — Localized fallback heading/)
    assert.ok(html.includes('Localized safe message') && !html.includes('private-failure-marker'))
    const cssPath = /rel="stylesheet" href="([^"]+)"/.exec(html)?.[1]
    const stylesheet = await fetch(origin + cssPath)
    assert.equal(stylesheet.status, 200)
    assert.match(stylesheet.headers.get('content-type'), /text\/css/)
    assert.match(await stylesheet.text(), /system-ui/)
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    server = undefined
    const dictionary = JSON.parse(readFileSync(join(app, 'i18n/fr.json'), 'utf8'))
    dictionary['http.error.message'] = 'Changed public fallback message'
    writeFileSync(join(app, 'i18n/fr.json'), JSON.stringify(dictionary))
    const secondBuild = brain(['build', '--site', 'https://example.test', '--json'], app)
    assert.notEqual(firstBuild.build, secondBuild.build, 'changed public fallback bytes must participate in the whole build signature')
    const changedManifest = JSON.parse(readFileSync(join(dist, '_server/manifest.json'), 'utf8'))
    assert.equal(changedManifest.errorFallback.locales.fr['http.error.message'], 'Changed public fallback message')
  } finally {
    if (server) await new Promise(resolve => { server.close(resolve); server.closeAllConnections() })
    rmSync(directory, { recursive: true, force: true })
  }
})
