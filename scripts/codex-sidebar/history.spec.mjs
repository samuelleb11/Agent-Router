import assert from 'node:assert/strict';
import { test } from 'node:test';
import { assertRolloutPreserved } from './history.mjs';

const before = [
  { timestamp: '2026-10-11T00:00:00.000Z', type: 'session_meta', payload: { id: 'thread-1', session_id: 'thread-1', history_mode: 'legacy', cwd: '/fixture' } },
  { timestamp: '2026-10-11T00:00:00.010Z', type: 'turn_context', payload: { turn_id: 'turn-1', model: 'team-model' } },
  { timestamp: '2026-10-11T00:00:00.020Z', type: 'event_msg', payload: { type: 'user_message', client_id: 'client-1', message: 'Read the file.\n', images: [], local_images: [], audio: [], local_audio: [], text_elements: [] } },
  { timestamp: '2026-10-11T00:00:00.030Z', type: 'response_item', payload: { type: 'function_call_output', call_id: 'call-1', output: 'FILE_CONTENT' } },
  { timestamp: '2026-10-11T00:00:00.040Z', type: 'event_msg', payload: { type: 'agent_message', message: 'Done', phase: null, memory_citation: null } },
];
const after = structuredClone(before);
after.forEach((row, ordinal) => { row.ordinal = ordinal; });
after[0].payload.history_mode = 'paginated';
after[2].payload = { type: 'item_completed', thread_id: 'thread-1', turn_id: 'turn-1', completed_at_ms: Date.parse(after[2].timestamp),
  item: { type: 'UserMessage', id: 'item-1', client_id: 'client-1', content: [{ type: 'text', text: 'Read the file.\n', text_elements: [] }] } };
after[4].payload = { type: 'item_completed', thread_id: 'thread-1', turn_id: 'turn-1', completed_at_ms: Date.parse(after[4].timestamp),
  item: { type: 'AgentMessage', id: 'item-2', content: [{ type: 'Text', text: 'Done' }] } };
const raw = rows => rows.map(row => JSON.stringify(row)).join('\n') + '\n';

test('preserves native migration and allows appended records', () => {
  assertRolloutPreserved(raw(before), raw(after));
  assertRolloutPreserved(raw(before), raw([...after, { timestamp: '2026-10-11T00:00:00.050Z', type: 'event_msg', ordinal: 5, payload: { type: 'task_complete' } }]));
});
test('rejects lost files and missing records', () => {
  assert.throws(() => assertRolloutPreserved(raw(before), undefined));
  assert.throws(() => assertRolloutPreserved(raw(before), raw(after.slice(0, -1))));
});
test('rejects changed thread identity and user content', () => {
  const changed = structuredClone(after); changed[0].payload.id = 'other-thread';
  assert.throws(() => assertRolloutPreserved(raw(before), raw(changed)));
  changed[0].payload.id = 'thread-1'; changed[2].payload.item.content[0].text = 'Changed';
  assert.throws(() => assertRolloutPreserved(raw(before), raw(changed)));
});
test('rejects changed tool data and reordered records', () => {
  const changed = structuredClone(after); changed[3].payload.output = 'CHANGED';
  assert.throws(() => assertRolloutPreserved(raw(before), raw(changed)));
  assert.throws(() => assertRolloutPreserved(raw(before), raw([after[0], after[1], after[3], after[2], after[4]])));
});
test('rejects migrated messages assigned to another turn', () => {
  const changed = structuredClone(after); changed[4].payload.turn_id = 'other-turn';
  assert.throws(() => assertRolloutPreserved(raw(before), raw(changed)));
});
test('keeps already paginated message metadata strict', () => {
  const changed = structuredClone(after); changed[2].payload.item.id = 'item-999';
  assert.throws(() => assertRolloutPreserved(raw(after), raw(changed)));
  changed[2].payload.item.id = 'item-1'; changed[2].payload.started_at_ms = 1;
  assert.throws(() => assertRolloutPreserved(raw(after), raw(changed)));
});
