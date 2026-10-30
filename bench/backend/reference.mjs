// Loads the explicitly selected reference application's real code, never its .env or developer database.
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'
import { join } from 'node:path'
const root=process.env.BRAIN_REFERENCE_API
const database=process.env.BRAIN_BENCH_DATABASE
if(!root || !/^postgres:\/\/brain_bench@127\.0\.0\.1:\d+\/reference_bench$/.test(database||'')) throw Error('Explicit reference source and disposable database required')
const require=createRequire(join(root,'package.json'))
const {Pool}=require('pg')
const pool=new Pool({connectionString:database,max:8})
const load=path=>import(pathToFileURL(join(root,path)))
const {migrate}=await load('src/lib/migrate.js')
const {seed}=await load('seeds/seed.js')
const {buildApp}=await load('src/app.js')
const {loadConfig}=await load('src/config.js')
await migrate(pool,()=>{})
const password=process.env.BRAIN_BENCH_PASSWORD
await seed(pool,{admin:password,candidate:password,company:password})
const config=loadConfig({databaseUrl:database,loginRateLimit:120,globalRateLimit:120,storageDir:process.env.BRAIN_BENCH_STORAGE,cvAi:{enabled:false},google:{},stripe:{},paypal:{},resendApiKey:'',mailer:{send:async()=>{throw Error('Benchmark mail is disabled')}},cookieSecure:true,corsOrigins:['https://benchmark.example.test']})
const app=await buildApp({config,pool,logger:false})
const url=await app.listen({host:'127.0.0.1',port:0})
const {rows}=await pool.query("SELECT title,description AS body FROM jobs WHERE status='published' ORDER BY id LIMIT 100")
console.log(JSON.stringify({event:'ready',url,rows}))
let closing=false
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,async()=>{if(closing)return;closing=true;await app.close();await pool.end();process.exit(0)})
