// SPDX-License-Identifier: GPL-3.0-only
// The audio worker: the infrasonic engine in WebAssembly. Messages that arrive while the
// module loads are kept and handed over once it runs.
import init from './pkg/infrasonic-engine-worker.js';

const early = [];
self.onmessage = (e) => early.push(e);
await init();
for (const e of early) {
  self.onmessage(e);
}
