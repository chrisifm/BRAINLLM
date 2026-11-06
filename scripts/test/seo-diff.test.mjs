import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, writeFileSync, mkdirSync, readFileSync, readdirSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {spawnSync} from 'node:child_process';
test('seo diff public dispatch preserves exits, strict flags and read-only sitemap output', { timeout: 45000 }, () => {
const binary = process.env.BRAIN_BIN;
assert.ok(binary, 'BRAIN_BIN must select the integrated binary explicitly');
const root = mkdtempSync(join(tmpdir(), 'brain-seo-diff-cli-'));
try {
  writeFileSync(join(root, 'tokens.cssbrain'), '');
  writeFileSync(join(root, 'brain.json'), '{"name":"coverage-fixture"}');
  mkdirSync(join(root, 'dist'));
  const sitemap = urls => `<urlset>${urls.map(u => `<url><loc>https://example.com${u}</loc></url>`).join('')}</urlset>`;
  writeFileSync(join(root, 'baseline.xml'), sitemap(['/keep','/lost']));
  const built = sitemap(['/keep']);
  writeFileSync(join(root, 'dist/sitemap.xml'), built);
  const run = args => spawnSync(resolve(binary), ['seo', 'diff', ...args], {cwd:root, encoding:'utf8',timeout:10000});
  for (const args of [['--from','baseline.xml','--json'],['--json','--from=baseline.xml']]) {
    const result=run(args); assert.equal(result.status,1,result.stderr || result.stdout);
    const report=JSON.parse(result.stdout); assert.equal(report.schemaVersion,1);
    assert.equal(report.missingCount,1); assert.deepEqual(report.missing,['/lost']);
  }
  const text=run(['--from','baseline.xml']); assert.equal(text.status,1); assert.match(text.stdout,/1 missing/);
  for (const args of [['--from'],['--from=baseline.xml','--from=other.xml'],['--from=baseline.xml','stray'],['--from=baseline.xml','--path=/keep'],['--from=baseline.xml','--json','--json']]) {
    const result=run(args); assert.equal(result.status,2,JSON.stringify({args,result}));
  }
  assert.equal(readFileSync(join(root,'dist/sitemap.xml'),'utf8'),built);
  assert.deepEqual(readdirSync(join(root,'dist')),['sitemap.xml']);
  writeFileSync(join(root,'dist/sitemap.xml'),sitemap(['/keep','/lost']));
  assert.equal(run(['--from=baseline.xml','--json']).status,0);
  console.log('seo diff public CLI: 9 cases passed; read-only output verified');
} finally { rmSync(root,{recursive:true,force:true}); }

});
