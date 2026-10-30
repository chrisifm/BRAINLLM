globalThis.events = []
try {
  const unused = [later]
  let later = 1
} catch (error) { globalThis.events.push(error.name) }
try {
  const unused = missingGlobal
} catch (error) { globalThis.events.push(error.name) }
try {
  const unused = 'x' in null
} catch (error) { globalThis.events.push(error.name) }
try {
  const unused = 1n + 2
} catch (error) { globalThis.events.push(error.name) }
process.stdout.write(JSON.stringify(globalThis.events))
