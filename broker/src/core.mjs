import { blake2b } from '@noble/hashes/blake2.js';
import { decodeAddress, encodeAddress } from '@polkadot/util-crypto';

export const GENESIS = '99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631';
export const PERSONAL = Buffer.from('ShieldedTab_C_v0');

export function refuseMainnet(value) {
  const s = String(value);
  let host = '';
  try { host = new URL(s).hostname.toLowerCase(); } catch {}
  if (/^(uview1|u1|zs1|t1|t3)/i.test(s) ||
      (host && (host === 'zec.rocks' || host.endsWith('.zec.rocks') && !host.startsWith('testnet.'))) ||
      /mainnet/i.test(s)) {
    throw new Error('mainnet value refused');
  }
  return s;
}

function field(bytes) {
  const b = Buffer.from(bytes);
  const len = Buffer.alloc(4);
  len.writeUInt32BE(b.length);
  return Buffer.concat([len, b]);
}

export function commitment(q) {
  refuseMainnet(q.address);
  if (q.network !== 'test') throw new Error('Zcash network must be test');
  const session = /^flop-devnet:([0-9a-f]{64}):([^:]+):([^:]+)$/.exec(q.session_id);
  if (!session || session[1] !== GENESIS) throw new Error('FLOP devnet session id refused');
  for (const address of session.slice(2)) {
    const decoded = decodeAddress(address, false, 42);
    if (encodeAddress(decoded, 42) !== address) throw new Error('non-canonical FLOP SS58 address');
  }
  const nonce = Buffer.from(q.nonce, 'hex');
  if (nonce.length !== 32 || !/^[0-9a-f]{64}$/.test(q.nonce)) throw new Error('nonce must be 32-byte lowercase hex');
  const amount = Buffer.alloc(8); amount.writeBigUInt64BE(BigInt(q.amount_zat));
  const expiry = Buffer.alloc(4); expiry.writeUInt32BE(Number(q.expiry_height));
  const input = Buffer.concat([
    field(Buffer.from([0])), field('test'), field(q.address), field(amount),
    field(q.session_id), field(nonce), field(expiry),
  ]);
  return Buffer.from(blake2b(input, { dkLen: 32, personalization: PERSONAL })).toString('hex');
}

export function parseMemo(text) {
  const m = /^STAB0:([0-9a-f]{64}):([0-9a-f]{64}):([0-9a-f]{32}):(0|[1-9][0-9]*):(0|[1-9][0-9]*)$/.exec(text);
  if (!m) throw new Error('malformed STAB0 memo');
  return { c: m[1], nonce: m[2], quote_id: m[3], amount_zat: m[4], expiry_height: m[5] };
}

export function matchPayment(quote, output, tipHeight, used = new Set()) {
  try {
    refuseMainnet(quote.address);
    if (quote.used) return { ok: false, reason: 'replay' };
    if (quote.session_id !== output.session_id) return { ok: false, reason: 'wrong_session' };
    if (Number(output.height) > Number(quote.expiry_height)) return { ok: false, reason: 'expired' };
    if (BigInt(output.value_zat) < BigInt(quote.amount_zat)) return { ok: false, reason: 'wrong_amount' };
    const memo = output.stab ?? parseMemo(output.memo ?? Buffer.from(output.memo_hex ?? '', 'hex').toString('utf8').replace(/\0+$/, ''));
    if (used.has(output.txid) || used.has(memo.quote_id)) return { ok: false, reason: 'replay' };
    if (memo.quote_id !== quote.quote_id) return { ok: false, reason: 'wrong_quote' };
    if (BigInt(memo.amount_zat) !== BigInt(quote.amount_zat) || Number(memo.expiry_height) !== Number(quote.expiry_height)) return { ok: false, reason: 'wrong_quote_fields' };
    const c = commitment({ ...quote, nonce: memo.nonce });
    if (c !== memo.c || c !== quote.c) return { ok: false, reason: 'wrong_commitment' };
    return { ok: true, memo, c };
  } catch (e) {
    return { ok: false, reason: /mainnet/.test(e.message) ? 'mainnet_refused' : 'malformed_memo' };
  }
}
