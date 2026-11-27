// SPDX-License-Identifier: GPL-3.0-only
// Starts the page program. A file of its own, not inline, so that the Content-Security-Policy
// allows only scripts of the player itself.
import init from './pkg/infrasonic-web.js';

init().catch((e) => {
  document.body.textContent = 'infrasonic could not start: ' + e;
});
