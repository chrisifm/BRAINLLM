// One opt-in startup watchdog. It cannot outlive this process or target any other process.
import { workerData, parentPort } from 'node:worker_threads'
import { writeSync } from 'node:fs'
const remaining = Math.max(0, Number(BigInt(workerData.deadline) - process.hrtime.bigint()) / 1e6)
setTimeout(() => {
  try { writeSync(2, '{"event":"startup-warmup-failed","code":"WARMUP_DEADLINE","private":true,"valuesCaptured":false}\n') } catch {}
  process.kill(process.pid, 'SIGKILL')
}, remaining)
parentPort.postMessage('armed')
