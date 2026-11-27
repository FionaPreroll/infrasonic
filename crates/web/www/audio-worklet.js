// SPDX-License-Identifier: GPL-3.0-only
// Plays interleaved stereo blocks sent by the audio worker through a MessagePort.
//
// worker → worklet: {t:"pcm", data: Float32Array}, {t:"flush", gen}, {t:"pause", v}
// worklet → worker: {t:"heard", heard, gen, starved} about every 20 ms; "heard" counts the
// frames played since the last flush, which drives the engine and its position.

class InfrasonicOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks = [];
    this.offset = 0;
    this.heard = 0;
    this.gen = 0;
    this.paused = false;
    this.starved = 0;
    this.last = 0;
    this.worker = null;
    this.port.onmessage = (e) => {
      if (e.data && e.data.port) {
        this.worker = e.data.port;
        this.worker.onmessage = (m) => this.receive(m.data);
        this.worker.start();
      }
    };
  }

  receive(d) {
    switch (d.t) {
      case 'pcm':
        this.chunks.push(d.data);
        break;
      case 'flush':
        this.chunks = [];
        this.offset = 0;
        this.heard = 0;
        this.gen = d.gen;
        this.report();
        break;
      case 'pause':
        this.paused = d.v;
        break;
    }
  }

  report() {
    if (this.worker) {
      this.worker.postMessage({ t: 'heard', heard: this.heard, gen: this.gen, starved: this.starved });
    }
    this.starved = 0;
    this.last = currentTime;
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const left = out[0];
    const right = out[1] || out[0];
    const n = left.length;
    let i = 0;
    if (!this.paused) {
      while (i < n && this.chunks.length > 0) {
        const c = this.chunks[0];
        while (i < n && this.offset < c.length) {
          left[i] = c[this.offset];
          right[i] = c[this.offset + 1];
          this.offset += 2;
          i++;
        }
        if (this.offset >= c.length) {
          this.chunks.shift();
          this.offset = 0;
        }
      }
      this.heard += i;
      if (i < n) {
        this.starved++;
      }
    }
    for (; i < n; i++) {
      left[i] = 0;
      right[i] = 0;
    }
    // Often while there is audio, rarely when idle.
    const every = this.chunks.length > 0 ? 0.02 : 0.25;
    if (currentTime - this.last >= every) {
      this.report();
    }
    return true;
  }
}

registerProcessor('infrasonic-output', InfrasonicOutput);
