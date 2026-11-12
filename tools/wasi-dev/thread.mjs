import { workerData } from 'node:worker_threads';
import { makeImports } from './common.mjs';
const { cfg, module, memory, stateBuf, tid, startArg } = workerData;
cfg.module = module;
const state = new Int32Array(stateBuf);
const { wasi, imports } = makeImports(cfg, memory, state, tid);
const inst = new WebAssembly.Instance(module, imports);
wasi.start({ exports: { memory, _start: () => inst.exports.wasi_thread_start(tid, startArg) } });
