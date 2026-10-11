import assert from 'node:assert/strict';

function keys(value, allowed) {
  for (const key of Object.keys(value)) assert.ok(allowed.includes(key), `Unsupported migrated history field ${key}`);
}

function records(raw) {
  return raw.trimEnd().split('\n').filter(Boolean).map(line => JSON.parse(line));
}

function normalize(rows, id, baseline = []) {
  let turn;
  return rows.map((source, index) => {
    const row = structuredClone(source);
    if (row.ordinal !== undefined) assert.equal(row.ordinal, index, 'History ordinals must retain record order');
    delete row.ordinal;
    if (row.type === 'session_meta') delete row.payload.history_mode;
    if (row.type === 'turn_context') turn = row.payload.turn_id;
    if (row.type !== 'event_msg') return row;
    let payload = row.payload;
    const legacy = baseline[index]?.payload?.type;
    if (payload.type === 'item_completed' && ['user_message', 'agent_message'].includes(legacy)) {
      keys(payload, ['type', 'thread_id', 'turn_id', 'item', 'completed_at_ms']);
      assert.equal(payload.thread_id, id, 'Migrated message changed thread identity');
      assert.equal(payload.turn_id, turn, 'Migrated message changed turn identity');
      assert.equal(payload.completed_at_ms, Date.parse(row.timestamp), 'Migrated message changed its timestamp');
      const item = payload.item;
      assert.equal(item.type, legacy === 'user_message' ? 'UserMessage' : 'AgentMessage');
      assert.match(item.id, /^item-\d+$/, 'Expected native display-item identity');
      assert.equal(item.content.length, 1, 'This text fixture must retain one message content block');
      const text = item.content[0];
      if (item.type === 'UserMessage') {
        keys(item, ['type', 'id', 'client_id', 'content']);
        keys(text, ['type', 'text', 'text_elements']);
        assert.equal(text.type, 'text');
        payload = { type: 'user_message', client_id: item.client_id ?? null, message: text.text,
          images: [], local_images: [], audio: [], local_audio: [], text_elements: text.text_elements ?? [] };
      } else {
        keys(item, ['type', 'id', 'content', 'phase', 'memory_citation']);
        keys(text, ['type', 'text']);
        assert.equal(text.type, 'Text');
        payload = { type: 'agent_message', message: text.text, phase: item.phase ?? null, memory_citation: item.memory_citation ?? null };
      }
    }
    if (payload.type === 'user_message') payload.client_id ??= null;
    row.payload = payload;
    return row;
  });
}

// The pinned native app-server migrates legacy display events to paginated
// history on restart. Keep every record, timestamp, payload and order; normalize
// only its observed transport metadata and equivalent text display events.
export function assertRolloutPreserved(beforeRaw, afterRaw) {
  assert.equal(typeof afterRaw, 'string', 'Saved rollout file disappeared');
  const before = records(beforeRaw);
  const after = records(afterRaw);
  assert.equal(before[0]?.type, 'session_meta');
  assert.equal(after[0]?.type, 'session_meta');
  const id = before[0].payload.id;
  assert.equal(after[0].payload.id, id, 'Saved thread identity changed');
  const oldMode = before[0].payload.history_mode;
  const newMode = after[0].payload.history_mode;
  assert.ok(oldMode === newMode || (oldMode === 'legacy' && newMode === 'paginated'), 'Unexpected history-mode change');
  assert.ok(after.length >= before.length, 'Saved rollout records disappeared');
  assert.deepEqual(normalize(after, id, before).slice(0, before.length), normalize(before, id), 'Saved rollout content or record order changed');
}
