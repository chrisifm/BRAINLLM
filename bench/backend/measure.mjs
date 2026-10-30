// Finite closed-loop transport measurement. The runner owns destinations, credentials and lifecycle.
import {performance} from 'node:perf_hooks'
export async function measure(request,{samples=100,warmup=5,concurrency=2}={}) {
  if(!Number.isInteger(samples)||samples<20||samples>1000||!Number.isInteger(warmup)||warmup<0||warmup>20||!Number.isInteger(concurrency)||concurrency<1||concurrency>8)throw Error('Invalid benchmark bounds')
  for(let i=0;i<warmup;i++)await request()
  let next=0,bytes=0
  const latency=[]
  const start=performance.now()
  await Promise.all(Array.from({length:concurrency},async()=>{
    for(;;){const index=next++;if(index>=samples)break;const at=performance.now();bytes+=await request();latency.push(performance.now()-at)}
  }))
  const elapsed=performance.now()-start
  latency.sort((a,b)=>a-b)
  const quantile=p=>latency[Math.min(latency.length-1,Math.ceil(p*latency.length)-1)]
  return {samples,warmup,concurrency,elapsedMs:elapsed,requestsPerSecond:samples*1000/elapsed,p50Ms:quantile(.5),p99Ms:quantile(.99),maxMs:latency.at(-1),responseBytes:bytes}
}
