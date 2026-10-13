// The terminal pane: xterm.js in front of a shell that the host runs in a pseudo-terminal.
import { invoke, Channel } from './vendor/tauri/core.js';

const term = new Terminal({
  fontFamily: '"Cascadia Mono", Consolas, monospace',
  fontSize: 13,
  cursorBlink: true,
  scrollback: 5000,
  theme: { background: '#14161a' },
});
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open(document.getElementById('term'));
window.__term = term; // for inspection from dev builds

let running = false;

async function start() {
  fit.fit();
  const output = new Channel();
  output.onmessage = (message) => {
    if (message instanceof ArrayBuffer) {
      term.write(new Uint8Array(message));
    } else if (message && message.exited) {
      running = false;
      term.write('\r\n\x1b[2m[shell exited - press Enter to start it again]\x1b[0m\r\n');
    }
  };
  // Input must flow from the first moment: the Windows console asks the terminal
  // for the cursor position as it starts and shows nothing until it gets the
  // answer, which xterm sends as input and which can arrive before this call returns.
  running = true;
  try {
    await invoke('terminal_start', { output, cols: term.cols, rows: term.rows });
  } catch (error) {
    running = false;
    term.write(`\r\n\x1b[31mcould not start a shell: ${error}\x1b[0m\r\n`);
  }
}

term.onData((data) => {
  if (running) invoke('terminal_write', { data });
  else if (data === '\r') start();
});
term.onResize(({ cols, rows }) => {
  if (running) invoke('terminal_resize', { cols, rows });
});
new ResizeObserver(() => fit.fit()).observe(document.getElementById('term'));

// Dragging the left edge resizes the pane. The pane's own left edge follows the
// pointer, so the pointer's x within the pane is how far it still has to move.
const grip = document.getElementById('grip');
let pendingDelta = null;
function sendDrag(done) {
  const delta = pendingDelta ?? 0;
  pendingDelta = null;
  invoke('terminal_drag', { delta: delta * devicePixelRatio, done });
}
grip.addEventListener('pointerdown', (event) => {
  grip.setPointerCapture(event.pointerId);
  grip.classList.add('dragging');
});
grip.addEventListener('pointermove', (event) => {
  if (!grip.hasPointerCapture(event.pointerId)) return;
  const first = pendingDelta === null;
  pendingDelta = event.clientX;
  if (first) requestAnimationFrame(() => sendDrag(false));
});
grip.addEventListener('pointerup', (event) => {
  grip.releasePointerCapture(event.pointerId);
  grip.classList.remove('dragging');
  sendDrag(true);
});

// The pane may be hidden at launch; a shell is only started once it is first shown.
if (document.visibilityState === 'visible') {
  start();
} else {
  document.addEventListener('visibilitychange', function shown() {
    if (document.visibilityState !== 'visible') return;
    document.removeEventListener('visibilitychange', shown);
    start();
  });
}
term.focus();
