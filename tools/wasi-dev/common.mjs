import fs from 'node:fs';
import { WASI } from 'node:wasi';
import { Worker } from 'node:worker_threads';
import { fileURLToPath } from 'node:url';

export function memoryLimits(buf) {
  // Разбор секции импорта: ищем память и её пределы.
  let p = 8;
  const u = () => { let r = 0, s = 0, b; do { b = buf[p++]; r |= (b & 0x7f) << s; s += 7; } while (b & 0x80); return r >>> 0; };
  const limits = () => { const flags = buf[p++]; const min = u(); const max = (flags & 1) ? u() : undefined; return { flags, min, max }; };
  while (p < buf.length) {
    const id = buf[p++]; const size = u(); const end = p + size;
    if (id === 2) {
      const n = u();
      for (let i = 0; i < n; i++) {
        const ml = u(); p += ml; const nl = u(); p += nl;
        const kind = buf[p++];
        if (kind === 0) u();
        else if (kind === 1) { p++; limits(); }
        else if (kind === 2) { const l = limits(); return { initial: l.min, maximum: l.max, shared: !!(l.flags & 2) }; }
        else if (kind === 3) { p += 2; }
        else if (kind === 4) { p++; u(); }
      }
    }
    p = end;
  }
  throw new Error('no memory import');
}

export function makeImports(cfg, memory, state, selfTid) {
  const wasi = new WASI({
    version: 'preview1', args: cfg.args, env: cfg.env, preopens: cfg.preopens, returnOnExit: false,
  });
  const imports = {
    env: { memory },
    wasi_snapshot_preview1: wasi.wasiImport,
    wasix_32v1: { fd_pipe: () => 52, proc_spawn3: () => 52 },
    wasi: {
      'thread-spawn': (startArg) => {
        const tid = Atomics.add(state, 0, 1) + 1;
        try {
          const w = new Worker(fileURLToPath(new URL('./thread.mjs', import.meta.url)), {
            workerData: { cfg, module: cfg.module, memory, stateBuf: state.buffer, tid, startArg },
            resourceLimits: { stackSizeMb: cfg.stackMb, maxOldGenerationSizeMb: 512 },
            stdout: false, stderr: false,
          });
          w.on('error', (e) => { console.error('thread', tid, 'error', e); process.exit(101); });
          w.unref();
        } catch (e) { console.error('spawn failed', e); return -6; }
        return tid;
      },
    },
  };
  return { wasi, imports };
}
