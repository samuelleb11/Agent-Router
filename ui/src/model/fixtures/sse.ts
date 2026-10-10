// New for agent-router: the wire framed as scripts/check-live-ui.js frames it (`replay`), for the
// specs that replay a stream through the session model. Not used by the console: a browser's
// EventSource does this framing itself.

/// One event off the wire: an unnamed one is a feed row ("message"), a named one says its name.
export interface WireEvent {
  readonly type: string;
  readonly data: string;
}

/// Server-sent events, as the check frames them (SE-C213): an event ends at a blank line; `event:`
/// names it, and one with no name is a message; `data:` lines join with newlines; a line opening
/// with ":" is a comment, which is all the keep-alive is. An event cut off before its blank line is
/// dropped. Like the check, the end of the text after a final newline counts as that blank line.
export function frames(text: string): WireEvent[] {
  const events: WireEvent[] = [];
  let type = '';
  let data: string[] = [];
  for (const line of text.split(/\r\n|\r|\n/)) {
    if (!line) {
      if (data.length) events.push({ type: type || 'message', data: data.join('\n') });
      type = '';
      data = [];
      continue;
    }
    if (line.startsWith(':')) continue;
    const at = line.indexOf(':');
    const field = at < 0 ? line : line.slice(0, at);
    const value = at < 0 ? '' : line.slice(at + 1).replace(/^ /, '');
    if (field === 'event') type = value;
    else if (field === 'data') data.push(value);
  }
  return events;
}
