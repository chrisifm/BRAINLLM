// Local fixture protocol exerciser, never an LLM or a measured agent result.
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
let input = ''
for await (const chunk of process.stdin) {
  input += chunk
  if (Buffer.byteLength(input) > 8 * 1024 * 1024) throw Error('Replay input exceeds limit')
}
const request = JSON.parse(input)
const replay = JSON.parse(await readFile(process.argv[2], 'utf8'))
for (const [path, content] of Object.entries(replay.files || {})) {
  const destination = resolve(request.workspace, path)
  if (!destination.startsWith(request.workspace + '/') || typeof content !== 'string') throw Error('Unsafe replay file')
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, content)
}
console.log(JSON.stringify({ replay: true, ...(replay.report || {}) }))
