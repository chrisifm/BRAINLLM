// Independent acceptance oracle over CDP events, not a WebSocket frame parser.
const fields = {
  ready: ['type', 'protocol', 'durable', 'resyncRequired'],
  resync: ['type', 'reason'],
  invalidate: ['type', 'resource', 'id', 'operation', 'eventId'],
  pong: ['type'],
  error: ['type', 'code'],
}
export function frameMetadata(response) {
  if ([8, 9, 10].includes(response.opcode)) return null
  if (response.opcode !== 1) throw Error('BROWSER_BINARY_NATIVE_FRAME')
  const text = response.payloadData
  if (typeof text !== 'string' || text.length > 4096 || /[^\x00-\x7f]/.test(text)) throw Error('BROWSER_SOCKET_FRAME_BOUND')
  let value
  try { value = JSON.parse(text) } catch { throw Error('BROWSER_SOCKET_PROTOCOL_JSON') }
  const allowed = value && !Array.isArray(value) && Object.hasOwn(fields, value.type) ? fields[value.type] : null
  if (!allowed || Object.keys(value).some(key => !allowed.includes(key))) throw Error('SOCKET_FRAME_MUST_NOT_CONTAIN_ROW_BODY')
  const uuid = text => typeof text === 'string' && /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(text)
  const shape = value.type === 'ready' ? value.protocol === 1 && value.durable === false && value.resyncRequired === true
    : value.type === 'resync' ? value.reason === 'joined'
    : value.type === 'pong' ? true
    : value.type === 'error' ? value.code === 'ROOM_FORBIDDEN'
    : typeof value.resource === 'string' && /^[a-z][a-z0-9_]{0,47}$/.test(value.resource) && uuid(value.id) && uuid(value.eventId) && ['create', 'update', 'delete'].includes(value.operation)
  if (!shape || Object.keys(value).length !== allowed.length) throw Error('BROWSER_SOCKET_PROTOCOL_SHAPE')
  return Object.freeze(value)
}
