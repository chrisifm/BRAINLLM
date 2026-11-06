// A real CLI gate over the backend-check harness's disposable Postgres. Fixtures are owned and always removed.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { join } from 'node:path'
import { binary, databaseURL, brain, gate, own, sleep, stop, write } from './lib.mjs'

const admin = new URL(databaseURL)
assert(['127.0.0.1', 'localhost', '[::1]'].includes(admin.hostname), 'The audit gate requires the disposable loopback database')
assert.equal(decodeURIComponent(admin.username), 'brain_test', 'Use the owned backend-check cluster, never an app database')
const psql = process.env.PG_BIN ? join(process.env.PG_BIN, 'psql') : 'psql'
const tag = randomUUID().replaceAll('-', '').slice(0, 12)
const db = 'audit_db_' + tag, foreignDB = 'control_db_' + tag
const actor = 'audit_role_' + tag, foreignRole = 'control_role_' + tag
const password = 'synthetic_password_' + tag, valueMarker = 'private_value_' + tag
const hiddenTable = 'hidden_control_' + tag, privateNamespace = 'private_control_' + tag
const extensionNamespace = 'audit_extension'
const ident = value => '"' + value.replaceAll('"', '""') + '"'
function environment(url) {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('PG') && !['DATABASE_URL', 'BRAIN_ENV'].includes(key)))
  return { ...env, PGHOST: url.hostname.replace(/^\[|\]$/g, ''), PGPORT: url.port || '5432', PGUSER: decodeURIComponent(url.username), PGPASSWORD: decodeURIComponent(url.password), PGDATABASE: decodeURIComponent(url.pathname.slice(1)), PGAPPNAME: 'brain:audit-gate', PGCONNECT_TIMEOUT: '3' }
}
function sql(query, url = admin) {
  const result = spawnSync(psql, ['-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1', '-c', query], { env: environment(url), encoding: 'utf8', timeout: 10000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] })
  // A fixture error must not print SQL literals, passwords, connection URLs or server messages.
  assert.equal(result.status, 0, 'Owned database-audit SQL fixture operation failed')
  return result.stdout.trim()
}
const appURL = new URL(admin); appURL.pathname = '/' + db; appURL.username = actor; appURL.password = password
const otherURL = new URL(admin); otherURL.pathname = '/' + foreignDB
const otherRoleURL = new URL(appURL); otherRoleURL.username = foreignRole
function invoke(app, args = [], overrides = {}) {
  const env = { ...environment(admin), BRAIN_NO_DELEGATE: '1', DATABASE_URL: appURL.href, ...overrides }
  for (const key of Object.keys(env)) if (env[key] === null) delete env[key]
  const result = spawnSync(binary, ['db', 'audit', ...args, '--json'], { cwd: app, env, encoding: 'utf8', timeout: 25000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] })
  const captured = (result.stdout || '') + (result.stderr || '')
  for (const secret of [password, valueMarker, appURL.href, foreignDB, foreignRole, hiddenTable, privateNamespace, 'NEVER_PRINT_SQL_LITERAL']) assert(!captured.includes(secret), 'Audit output included a private fixture marker')
  assert(!result.error && !result.signal, 'Audit CLI did not finish within its process bound')
  assert(!result.stderr.trim(), 'Audit CLI wrote unexpected stderr')
  let report
  try { report = JSON.parse(result.stdout) } catch { throw Error('Audit CLI did not return one valid JSON document') }
  return { status: result.status, report }
}
function snapshot(url) {
  return sql("SELECT jsonb_build_object('overrides',(SELECT coalesce(jsonb_agg(to_jsonb(x) ORDER BY setdatabase,setrole),'[]') FROM pg_db_role_setting x),'objects',(SELECT coalesce(jsonb_agg(jsonb_build_array(n.nspname,c.relname,c.relkind) ORDER BY n.nspname,c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT LIKE 'pg_%' AND n.nspname<>'information_schema'),'settings',(SELECT jsonb_agg(jsonb_build_array(name,reset_val) ORDER BY name) FROM pg_settings WHERE name IN ('work_mem','statement_timeout','lock_timeout','idle_in_transaction_session_timeout')))::text", url)
}
await gate('database-audit', 150000, async directory => {
  let createdDB = false, createdOtherDB = false, createdActor = false, createdForeignRole = false
  const held = []
  try {
    sql(`CREATE ROLE ${ident(actor)} LOGIN PASSWORD '${password}'`); createdActor = true
    sql(`CREATE ROLE ${ident(foreignRole)} LOGIN`); createdForeignRole = true
    sql(`CREATE DATABASE ${ident(db)} OWNER ${ident(actor)} TEMPLATE template0`); createdDB = true
    sql(`CREATE DATABASE ${ident(foreignDB)} TEMPLATE template0`); createdOtherDB = true
    const fixtureAdmin = new URL(admin); fixtureAdmin.pathname = '/' + db
    sql(`CREATE TABLE ${ident(hiddenTable)} (value text); INSERT INTO ${ident(hiddenTable)} VALUES ('${valueMarker}'); REVOKE ALL ON ${ident(hiddenTable)} FROM PUBLIC; CREATE SCHEMA ${ident(privateNamespace)}; REVOKE ALL ON SCHEMA ${ident(privateNamespace)} FROM PUBLIC`, fixtureAdmin)
    sql(`CREATE TABLE outside_control (value text); INSERT INTO outside_control VALUES ('${valueMarker}')`, otherURL)
    sql(`CREATE TABLE readable_one (id integer PRIMARY KEY, value text); INSERT INTO readable_one SELECT n,'${valueMarker}' FROM generate_series(1,12) n; CREATE TABLE readable_two (id integer PRIMARY KEY); CREATE TABLE readable_three (id integer PRIMARY KEY); ANALYZE readable_one`, appURL)
    // Query texts include a distinctive value and foreign-role work. Neither is allowed into the report.
    sql(`SELECT '${valueMarker}'::text`, appURL)
    sql('SELECT count(*) FROM pg_catalog.pg_class', otherRoleURL)
    // A non-public extension namespace exercises catalog lookup and identifier quoting without assuming public.
    const available = sql("SELECT count(*) FROM pg_available_extensions WHERE name='pg_stat_statements'", fixtureAdmin) === '1'
    const preloaded = sql("SELECT current_setting('shared_preload_libraries') LIKE '%pg_stat_statements%'", fixtureAdmin) === 't'
    brain(['new', 'audit-fixture', '--static'], directory, { BRAIN_NO_DELEGATE: '1', DATABASE_URL: '' })
    const app = join(directory, 'audit-fixture')
    write(app, '.brain_env', '[development]\nDATABASE_URL=invalid-file-url-NEVER_PRINT_SQL_LITERAL\n[production]\nDATABASE_URL=invalid-production-url-NEVER_PRINT_SQL_LITERAL\n')
    const absent = invoke(app)
    assert.equal(absent.status, 0); assert.equal(absent.report.statements.status, 'unavailable'); assert.equal(absent.report.statements.reason, 'extension-not-installed')
    if (available) sql(`CREATE SCHEMA ${ident(extensionNamespace)}; GRANT USAGE ON SCHEMA ${ident(extensionNamespace)} TO PUBLIC; CREATE EXTENSION pg_stat_statements WITH SCHEMA ${ident(extensionNamespace)}`, fixtureAdmin)
    const before = snapshot(fixtureAdmin), otherBefore = snapshot(otherURL)
    const originalStatementReset = sql("SELECT reset_val FROM pg_settings WHERE name='statement_timeout'", appURL)
    for (const url of [appURL, otherRoleURL, new URL(appURL)]) {
      if (held.length === 2) url.pathname = '/' + foreignDB
      held.push(own(psql, ['-X', '-q', '-c', 'SELECT pg_sleep(60)'], directory, environment(url)))
    }
    const deadline = Date.now() + 5000
    while (sql("SELECT count(*) FROM pg_stat_activity WHERE application_name='brain:audit-gate' AND query='SELECT pg_sleep(60)'", fixtureAdmin) !== '3') {
      assert(Date.now() < deadline, 'Scoped connection fixtures did not become ready'); await sleep(50)
    }
    const { status, report } = invoke(app, ['--limit', '1', '--env', 'production'])
    assert.equal(status, 0); assert.equal(report.ok, true); assert.equal(report.readOnly, true); assert.equal(report.applied, false)
    assert.equal(report.scope.database, db); assert.equal(report.scope.role, actor); assert.equal(report.scope.transactionReadOnly, true)
    assert.equal(report.scope.statementTimeout, '2s'); assert.equal(report.scope.lockTimeout, '250ms')
    assert.equal(report.environment, 'production'); assert.equal(report.tables.observations.length, 1); assert.equal(report.tables.truncated, true)
    assert.equal(report.indexes.observations.length, 1); assert.equal(report.indexes.truncated, true)
    assert.equal(report.connections.observations.clientBackends, 1, 'Other databases and roles must not count as current-role activity')
    assert(report.settings.observations.every(row => !('sourcefile' in row)))
    const statementSetting = report.settings.observations.find(row => row.name === 'statement_timeout')
    assert.equal(statementSetting.sourceResetValue, originalStatementReset); assert.equal(statementSetting.actualValue, '2000'); assert.equal(statementSetting.auditSessionOverride, true)
    assert.equal(report.connectionBudget.actualMemoryUsed, null); assert.equal(report.connectionBudget.oomCause, null)
    for (const proposal of report.recommendations) {
      assert.equal(proposal.applied, false); assert.equal(proposal.requiresOperatorApproval, true)
      if (proposal.sql) assert(proposal.sql.startsWith(`ALTER ROLE ${ident(actor)} IN DATABASE ${ident(db)} SET `))
    }
    if (available && preloaded) {
      assert.equal(report.statements.status, 'available'); assert(report.statements.observations.length > 0)
      const ownIds = new Set(JSON.parse(sql(`SELECT coalesce(jsonb_agg(queryid::text),'[]') FROM ${ident(extensionNamespace)}.pg_stat_statements(false) WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND userid=(SELECT oid FROM pg_roles WHERE rolname='${actor}')`, fixtureAdmin)))
      assert(report.statements.observations.every(row => ownIds.has(row.queryId) && !('query' in row) && !('userid' in row) && !('dbid' in row)))
    } else {
      assert.equal(report.statements.status, 'unavailable'); assert.equal(report.statements.reason, available ? 'not-preloaded' : 'extension-not-installed')
    }
    assert.equal(report.queryTextCaptured, false); assert.equal(report.valuesCaptured, false); assert.equal(report.foreignIdentitiesCaptured, false)
    assert.equal(snapshot(fixtureAdmin), before); assert.equal(snapshot(otherURL), otherBefore)
    assert.equal(sql('SELECT count(*) FROM readable_one', appURL), '12')
    // No process DATABASE_URL: canonical selected app section must win; shared fallback is exercised separately.
    write(app, '.brain_env', `DATABASE_URL=invalid-shared-NEVER_PRINT_SQL_LITERAL\n[development]\nDATABASE_URL=${appURL.href}\n`)
    assert.equal(invoke(app, [], { DATABASE_URL: null }).status, 0)
    write(app, '.brain_env', `DATABASE_URL=${appURL.href}\n`)
    assert.equal(invoke(app, [], { DATABASE_URL: null }).status, 0)
    const outsideApp = invoke(directory)
    assert.equal(outsideApp.status, 2); assert.equal(outsideApp.report.error.code, 'DB_AUDIT_APP_REQUIRED')
    write(app, '.brain_env', '')
    const unconfigured = invoke(app, [], { DATABASE_URL: null })
    assert.equal(unconfigured.status, 2); assert.equal(unconfigured.report.error.code, 'DB_AUDIT_DATABASE_REQUIRED')
    for (const args of [['--apply'], ['--limit','0'], ['--limit','201'], ['--env','../other'], ['--limit','1','--limit','2'], ['--url','NEVER_PRINT_SQL_LITERAL']]) {
      const bad = invoke(app, args, { DATABASE_URL: 'invalid-NEVER_PRINT_SQL_LITERAL' })
      assert.equal(bad.status, 2); assert.equal(bad.report.error.code, 'DB_AUDIT_ARGUMENTS')
    }
    const invalid = invoke(app, [], { DATABASE_URL: 'invalid-NEVER_PRINT_SQL_LITERAL' })
    assert.equal(invalid.status, 1); assert.equal(invalid.report.error.code, 'DB_AUDIT_CONNECTION_POLICY')
    const missingDatabase = new URL(appURL); missingDatabase.pathname = '/missing_NEVER_PRINT_SQL_LITERAL'
    const disconnected = invoke(app, [], { DATABASE_URL: missingDatabase.href })
    assert.equal(disconnected.status, 1); assert.equal(disconnected.report.error.code, 'DB_AUDIT_CONNECTION_UNAVAILABLE'); assert.equal(disconnected.report.error.sqlstate, '3D000')
    if (available) {
      sql(`REVOKE EXECUTE ON FUNCTION ${ident(extensionNamespace)}.pg_stat_statements(boolean) FROM PUBLIC`, fixtureAdmin)
      try {
        const noStatements = invoke(app)
        assert.equal(noStatements.status, 0); assert.equal(noStatements.report.statements.status, 'unavailable'); assert.equal(noStatements.report.statements.reason, 'permission-denied')
      } finally { sql(`GRANT EXECUTE ON FUNCTION ${ident(extensionNamespace)}.pg_stat_statements(boolean) TO PUBLIC`, fixtureAdmin) }
    }
    const insecureRemote = new URL(appURL)
    insecureRemote.hostname = '192.0.2.1'
    insecureRemote.searchParams.set('sslmode', 'prefer')
    const noTLS = invoke(app, ['--env', 'production'], { DATABASE_URL: insecureRemote.href })
    assert.equal(noTLS.status, 1); assert.equal(noTLS.report.error.code, 'DB_AUDIT_CONNECTION_POLICY')
    sql('REVOKE SELECT ON pg_catalog.pg_extension FROM PUBLIC', fixtureAdmin)
    try {
      const noExtensionMetadata = invoke(app)
      assert.equal(noExtensionMetadata.status, 0); assert.equal(noExtensionMetadata.report.statements.status, 'unavailable'); assert.equal(noExtensionMetadata.report.statements.reason, 'permission-denied')
    } finally { sql('GRANT SELECT ON pg_catalog.pg_extension TO PUBLIC', fixtureAdmin) }
    // A denied catalog section is explicit partial evidence, not a success or a raw server error.
    sql('REVOKE SELECT ON pg_catalog.pg_stat_user_tables FROM PUBLIC', fixtureAdmin)
    try {
      const denied = invoke(app)
      assert.equal(denied.status, 1); assert.equal(denied.report.ok, false)
      assert.equal(denied.report.tables.status, 'unavailable'); assert.equal(denied.report.tables.reason, 'permission-denied'); assert.equal(denied.report.tables.sqlstate, '42501')
      assert.equal(denied.report.connections.status, 'available'); assert.equal(denied.report.indexes.status, 'available')
    } finally { sql('GRANT SELECT ON pg_catalog.pg_stat_user_tables TO PUBLIC', fixtureAdmin) }
    assert.equal(snapshot(fixtureAdmin), before); assert.equal(snapshot(otherURL), otherBefore)
    console.log('database-audit: scoped DB/role, read-only persistence, limits, config precedence, TLS, permissions and private markers verified; statement sampling ' + (available && preloaded ? 'verified with preload and non-public extension namespace' : 'explicitly unavailable without optional server capability'))
  } finally {
    await Promise.all(held.map(({ child }) => stop(child)))
    if (createdDB) sql(`DROP DATABASE ${ident(db)} WITH (FORCE)`)
    if (createdOtherDB) sql(`DROP DATABASE ${ident(foreignDB)} WITH (FORCE)`)
    if (createdActor) sql(`DROP ROLE ${ident(actor)}`)
    if (createdForeignRole) sql(`DROP ROLE ${ident(foreignRole)}`)
  }
})
