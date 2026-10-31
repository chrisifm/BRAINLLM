import test from 'node:test'
import assert from 'node:assert/strict'
import { frameMetadata } from './frame-metadata.mjs'
const text = value => ({ opcode: 1, payloadData: JSON.stringify(value) })
test('native metadata audit ignores valid transport control frames', () => {
  for (const opcode of [8, 9, 10]) assert.equal(frameMetadata({ opcode, payloadData: 'control bytes are not JSON' }), null)
})
test('native metadata is bounded, immutable and excludes row bodies', () => {
  const value = { type: 'invalidate', resource: 'posts', id: '00000000-0000-4000-8000-000000000001', operation: 'update', eventId: '00000000-0000-4000-8000-000000000002' }
  const frame = frameMetadata(text(value))
  assert.deepEqual(frame, value)
  assert(Object.isFrozen(frame))
  assert.throws(() => frameMetadata(text({ type: 'ready', protocol: { secret: 'synthetic' }, durable: false, resyncRequired: true })), /BROWSER_SOCKET_PROTOCOL_SHAPE/)
  assert.throws(() => frameMetadata(text({ ...value, id: { secret: 'synthetic' } })), /BROWSER_SOCKET_PROTOCOL_SHAPE/)
  for (const extra of [{ data: { title: 'synthetic-private-field' } }, { owner: 'synthetic-owner' }, { token: 'synthetic-token' }]) {
    assert.throws(() => frameMetadata(text({ ...value, ...extra })), /SOCKET_FRAME_MUST_NOT_CONTAIN_ROW_BODY/)
  }
  assert.throws(() => frameMetadata({ opcode: 2, payloadData: '{}' }), /BROWSER_BINARY_NATIVE_FRAME/)
  assert.throws(() => frameMetadata({ opcode: 1, payloadData: 'x'.repeat(4097) }), /BROWSER_SOCKET_FRAME_BOUND/)
  assert.throws(() => frameMetadata({ opcode: 1, payloadData: '{' }), /BROWSER_SOCKET_PROTOCOL_JSON/)
  assert.throws(() => frameMetadata(text({ type: '__proto__' })), /SOCKET_FRAME_MUST_NOT_CONTAIN_ROW_BODY/)
  assert.throws(() => frameMetadata(text([])), /SOCKET_FRAME_MUST_NOT_CONTAIN_ROW_BODY/)
  assert.throws(() => frameMetadata({ opcode: 1, payloadData: '{"type":"pong","__proto__":{}}' }), /SOCKET_FRAME_MUST_NOT_CONTAIN_ROW_BODY/)
})
