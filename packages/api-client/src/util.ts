/**
 * Deterministic, ASCII-safe string hash used for stable idempotency keys.
 *
 * Uses a 64-bit cyrb53 variant so it works in both Node and browser
 * runtimes without relying on crypto.subtle availability.
 */
export function stableHash(input: string): string {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < input.length; i++) {
    const ch = input.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2_654_435_761);
    h2 = Math.imul(h2 ^ ch, 1_597_334_677);
  }
  h1 =
    Math.imul(h1 ^ (h1 >>> 16), 2_246_822_507) ^
    Math.imul(h2 ^ (h2 >>> 13), 3_266_489_909);
  h2 =
    Math.imul(h2 ^ (h2 >>> 16), 2_246_822_507) ^
    Math.imul(h1 ^ (h1 >>> 13), 3_266_489_909);
  return (
    (h1 >>> 0).toString(16).padStart(8, "0") +
    (h2 >>> 0).toString(16).padStart(8, "0")
  );
}
