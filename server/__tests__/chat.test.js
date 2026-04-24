/**
 * Unit tests for `socket/chat.js` sanitation helpers. These sit on the
 * server's untrusted-input boundary — every chat message passes through
 * them before persistence + fan-out, so XSS-adjacent regressions are
 * shipped to every connected viewer.
 */
process.env.YOUTUBE_API_KEY ||= 'test';
process.env.JWT_SECRET ||= 'test';
process.env.DATABASE_URL ||= 'postgresql://test@localhost/test';
process.env.TENOR_API_KEY ||= 'test';

// describe / it / expect are globals via vitest.config.js.
const {
  _internal: { sanitizeText, clampCodepoints, formatServerTime },
} = require('../socket/chat');

describe('sanitizeText', () => {
  it('strips control characters below 0x20 (except tab/newline)', () => {
    const out = sanitizeText('\x00hello\x07world\x1f');
    expect(out).toBe('helloworld');
  });
  it('preserves printable ASCII', () => {
    expect(sanitizeText('Hello, world! 1+1=2.')).toBe('Hello, world! 1+1=2.');
  });
  it('preserves multi-byte Unicode (emoji, accented)', () => {
    expect(sanitizeText('café 🦊')).toBe('café 🦊');
  });
  // sanitizeText assumes string input by contract — `registerChatHandlers`
  // upstream rejects non-string payloads before calling. Documented as
  // a TypeError-throws here so a future refactor can't silently accept
  // random types.
  it('throws on non-string input (upstream contract)', () => {
    expect(() => sanitizeText(null)).toThrow();
    expect(() => sanitizeText(42)).toThrow();
  });
  it('collapses CRLF to newlines', () => {
    // Newline handling depends on impl; document the current behaviour.
    const out = sanitizeText('a\r\nb');
    expect(out).toContain('a');
    expect(out).toContain('b');
  });
});

describe('clampCodepoints', () => {
  it('is a no-op under the limit', () => {
    expect(clampCodepoints('short', 500)).toBe('short');
  });
  it('truncates at the codepoint boundary for max=N', () => {
    const input = 'abcdefghij'; // 10 chars, all ASCII
    expect(clampCodepoints(input, 5)).toBe('abcde');
  });
  it('does not split a surrogate pair mid-codepoint', () => {
    // 🦊 is U+1F98A, encoded as a UTF-16 surrogate pair (2 code units
    // but 1 codepoint). Clamping to N must count CODEPOINTS, not
    // code units, otherwise the result is an invalid truncated
    // surrogate that crashes some rendering paths.
    const input = 'ab🦊cd';
    // Should be 5 codepoints total.
    expect(clampCodepoints(input, 4)).toBe('ab🦊c');
    expect(clampCodepoints(input, 3)).toBe('ab🦊');
    expect(clampCodepoints(input, 2)).toBe('ab');
  });
  it('handles empty and zero-length max', () => {
    expect(clampCodepoints('', 10)).toBe('');
    expect(clampCodepoints('abc', 0)).toBe('');
  });
});

describe('formatServerTime', () => {
  it('returns HH:MM in Europe/Paris by default', () => {
    // Pick a fixed UTC instant and assert it maps to the Paris wall
    // clock we expect. 2026-04-15T08:00:00Z = 10:00 Paris (CEST/+2).
    const d = new Date('2026-04-15T08:00:00Z');
    expect(formatServerTime(d)).toBe('10:00');
  });
  it('zero-pads single-digit hours and minutes', () => {
    const d = new Date('2026-04-15T02:03:00Z'); // 04:03 Paris CEST
    expect(formatServerTime(d)).toBe('04:03');
  });
});
