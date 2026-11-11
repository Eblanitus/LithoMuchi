// Запуск обычной wasm32-wasip1 программы (тесты, песочница) под Node WASI.
import fs from 'node:fs';
import { WASI } from 'node:wasi';
const [wasmPath, ...args] = process.argv.slice(2);
const preopens = {};
for (const d of (process.env.WASI_DIRS || '/tmp').split(':')) if (d) preopens[d] = d;
if (process.env.WASI_CWD) preopens['.'] = process.env.WASI_CWD;
const env = {};
for (const k of Object.keys(process.env)) if (k.startsWith('WASIENV_')) env[k.slice(8)] = process.env[k];
const wasi = new WASI({ version: 'preview1', args: [wasmPath, ...args], env, preopens, returnOnExit: false });
const module = new WebAssembly.Module(fs.readFileSync(wasmPath));
const inst = new WebAssembly.Instance(module, wasi.getImportObject());
wasi.start(inst);
