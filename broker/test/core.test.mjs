import test from 'node:test';
import assert from 'node:assert/strict';
import { commitment, matchPayment, parseMemo, refuseMainnet } from '../src/core.mjs';
import { buildBatch } from '../src/devnet.mjs';

const q = {
  network: 'test', address: 'ztestsapling1vector0nly', amount_zat: '150000',
  session_id: 'flop-devnet:99c172c095340ace980631f9123544c6bebd4839f3437e463533422d8be2d631:5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY:5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty',
  nonce: '11'.repeat(32), quote_id: '22'.repeat(16), expiry_height: 4200000,
};
q.c = commitment(q);
const output = { txid: 'aa', height: 4199990, value_zat: '150000', session_id: q.session_id, memo: `STAB0:${q.c}:${q.nonce}:${'22'.repeat(16)}:150000:4200000` };

test('commitment matches SPEC vector', () => assert.equal(q.c, '2cd62794f5b5bb726e0d2bfaebfb9b8a4f0ac007518e1a1693fff55efbd85bd3'));
test('accepts a matching payment', () => assert.equal(matchPayment(q, output, 4199999).ok, true));
test('rejects an underpayment', () => assert.equal(matchPayment(q, { ...output, value_zat: '149999' }, 1).reason, 'wrong_amount'));
test('rejects payment mined after expiry', () => assert.equal(matchPayment(q, { ...output, height: 4200001 }, 4200100).reason, 'expired'));
test('accepts on-time payment scanned after expiry', () => assert.equal(matchPayment(q, output, 4200100).ok, true));
test('rejects a replay', () => assert.equal(matchPayment(q, output, 1, new Set(['aa'])).reason, 'replay'));
test('rejects a quote already marked used', () => assert.equal(matchPayment({ ...q, used: 'old-tx' }, output, 1).reason, 'replay'));
test('rejects a wrong session', () => assert.equal(matchPayment(q, { ...output, session_id: 'other' }, 1).reason, 'wrong_session'));
test('rejects malformed memo', () => assert.equal(matchPayment(q, { ...output, memo: 'STAB0:nope' }, 1).reason, 'malformed_memo'));
test('refuses mainnet values', () => { for (const s of ['uview1abc', 'u1abc', 'zs1abc', 't1abc', 't3abc', 'https://zec.rocks:443']) assert.throws(() => refuseMainnet(s)); });
test('refuses mainnet endpoints with paths and wrong FLOP genesis', () => {
  assert.throws(() => refuseMainnet('https://zec.rocks/rpc'));
  assert.throws(() => commitment({ ...q, session_id: q.session_id.replace('99c172', '00c172') }));
});
test('memo parser returns exact fields', () => assert.deepEqual(parseMemo(output.memo), { c: q.c, nonce: q.nonce, quote_id: '22'.repeat(16), amount_zat: '150000', expiry_height: '4200000' }));
test('rejects memo amount substitution', () => assert.equal(matchPayment(q, { ...output, memo: output.memo.replace(':150000:', ':150001:') }, 1).reason, 'wrong_quote_fields'));
test('rejects memo expiry substitution', () => assert.equal(matchPayment(q, { ...output, memo: output.memo.replace(':4200000', ':4200001') }, 1).reason, 'wrong_quote_fields'));
test('rejects noncanonical SS58 session owner', () => assert.throws(() => commitment({ ...q, session_id: q.session_id.replace('5Grwva', 'owner-') })));
test('rejects SS58 checksum mutation', () => assert.throws(() => commitment({ ...q, session_id: q.session_id.replace('KutQY', 'KutQX') })));
test('rejects SS58 network prefix mutation', () => assert.throws(() => commitment({ ...q, session_id: q.session_id.replace('5Grwva', '15oF4') })));
test('rejects memo with uppercase commitment', () => assert.throws(() => parseMemo(output.memo.replace(q.c, q.c.toUpperCase()))));
test('rejects memo leading zero amount', () => assert.throws(() => parseMemo(output.memo.replace(':150000:', ':0150000:'))));
test('rejects memo leading zero expiry', () => assert.throws(() => parseMemo(output.memo.replace(':4200000', ':04200000'))));
test('rejects memo field overflow', () => assert.throws(() => parseMemo(`${output.memo}:extra`)));
test('rejects changed nonce commitment', () => assert.equal(matchPayment(q, { ...output, memo: output.memo.replace(q.nonce, '12'.repeat(32)) }, 1).reason, 'wrong_commitment'));
test('rejects changed C', () => assert.equal(matchPayment(q, { ...output, memo: output.memo.replace(q.c, '00'.repeat(32)) }, 1).reason, 'wrong_commitment'));
test('rejects changed quote id', () => assert.equal(matchPayment(q, { ...output, memo: output.memo.replace('22'.repeat(16), '23'.repeat(16)) }, 1).reason, 'wrong_quote'));
test('rejects transaction replay even with a fresh used set', () => assert.equal(matchPayment(q, output, 1, new Set(['aa'])).reason, 'replay'));
test('rejects malformed amount type', () => assert.equal(matchPayment(q, { ...output, value_zat: 'not-zat' }, 1).reason, 'malformed_memo'));
test('rejects zero height output after nonzero expiry', () => assert.equal(matchPayment(q, { ...output, height: 4200001 }, 1).reason, 'expired'));
test('rejects malformed session shape', () => assert.throws(() => commitment({ ...q, session_id: 'flop-devnet:wrong' })));
test('rejects mainnet tag embedded in session id', () => assert.throws(() => commitment({ ...q, session_id: q.session_id.replace('flop-devnet', 'flop-mainnet') })));
test('builds dry-run batch against a fake API using metadata argument order', () => {
  const calls = [];
  const api = { tx: {
    flopSessionKeys: { createSession: (...args) => { calls.push(['createSession', ...args]); return 'create'; } },
    system: { remarkWithEvent: (...args) => { calls.push(['remarkWithEvent', ...args]); return 'remark'; } },
    utility: { batchAll: (...args) => ({ method: { toHex: () => '0x0102' }, calls: args }) },
  } };
  const built = buildBatch(api, { delegate: 'delegate', spendingCap: 8n, durationBlocks: 90, c: q.c });
  assert.deepEqual(calls[0], ['createSession', 'delegate', 8n, 8n, 8n, 90, []]);
  assert.equal(calls[1][1], `0x${Buffer.from(`STAB0:${q.c}`).toString('hex')}`);
  assert.deepEqual(built.batch.calls, [['create', 'remark']]);
});
