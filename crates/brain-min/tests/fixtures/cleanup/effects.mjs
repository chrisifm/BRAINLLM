globalThis.events = []
function effect(name) {
  globalThis.events.push(name)
  return { get value() { globalThis.events.push('getter'); return 1 } }
}
const unusedLiteral = 'PRIVATE_BODY_MARKER_NOT_A_CREDENTIAL'
const unusedCall = effect('call')
const unusedGetter = effect('source').value
const { field: unusedPattern = effect('default') } = {
  get field() { globalThis.events.push('destructure'); return undefined }
}
const [unusedIteration] = {
  [Symbol.iterator]() {
    globalThis.events.push('iterator')
    return [1][Symbol.iterator]()
  }
}
const unusedCoercion = {
  [Symbol.toPrimitive]() { globalThis.events.push('coercion'); return 2 }
} + 1
process.stdout.write(JSON.stringify(globalThis.events))
