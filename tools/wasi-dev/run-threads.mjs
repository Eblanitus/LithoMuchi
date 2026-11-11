// Запуск wasm32-wasip1-threads программы (rustc.wasm) под Node WASI.
import fs from 'node:fs';
import { memoryLimits, makeImports } from './common.mjs';
const [wasmPath, ...args] = process.argv.slice(2);
const buf = fs.readFileSync(wasmPath);
const module = new WebAssembly.Module(buf);
const lim = memoryLimits(buf);
const memory = new WebAssembly.Memory(lim);
const state = new Int32Array(new SharedArrayBuffer(16));
const preopens = {};
for (const d of (process.env.WASI_DIRS || '/tmp').split(':')) if (d) preopens[d] = d;
const env = { RUST_MIN_STACK: '16777216', TMPDIR: '/tmp', HOME: '/tmp', PATH: '/usr/bin:/bin' };
for (const k of Object.keys(process.env)) if (k.startsWith('WASIENV_')) env[k.slice(8)] = process.env[k];
const cfg = { args: [wasmPath, ...args], env, preopens, module, stackMb: 512 };
const { wasi, imports } = makeImports(cfg, memory, state, 0);
const inst = new WebAssembly.Instance(module, imports);
wasi.start({ exports: { memory, _start: inst.exports._start } });
