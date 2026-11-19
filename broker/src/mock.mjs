import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export class MockFlopAdapter {
  constructor(path) { this.path = path; }
  async openSession({ owner, delegate, c, sessionId }) {
    const state = await this.#read();
    state.sessions[sessionId] = { owner, delegate, active: true };
    state.remarks.push({ owner, bytes: `STAB0:${c}` });
    await this.#write(state);
    return { mock: true, block: state.remarks.length };
  }
  async verify({ owner, delegate, c, sessionId }) {
    const state = await this.#read();
    return Boolean(state.sessions[sessionId]?.active && state.sessions[sessionId].owner === owner && state.sessions[sessionId].delegate === delegate && state.remarks.some(r => r.owner === owner && r.bytes === `STAB0:${c}`));
  }
  async #read() { try { return JSON.parse(await readFile(this.path, 'utf8')); } catch { return { sessions: {}, remarks: [] }; } }
  async #write(s) { const path = this.path instanceof URL ? fileURLToPath(this.path) : this.path; await mkdir(dirname(path), { recursive: true, mode: 0o700 }); await writeFile(path, JSON.stringify(s, null, 2), { mode: 0o600 }); }
}
