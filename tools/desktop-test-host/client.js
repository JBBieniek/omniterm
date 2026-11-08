import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';

export async function inspectReady(app) {
  for (let attempt = 0; attempt < 50; attempt++) {
    const response = await app.request('inspect');
    const state = response.result;
    if (state && state.rendered.rendered_revision === state.layout.revision) return state;
    if (response.error && !['window_missing', 'bundled_content_required'].includes(response.error)) {
      throw new Error(`Native inspection failed: ${response.error}`);
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error('Native UI did not finish loading');
}

// Separate test-host client. Tokens travel over inherited pipes, never argv, env,
// URLs, logs or the renderer; no HTTP/CDP debugging listener is started.
export async function launchAutomation(binary, { hello } = {}) {
  const token = randomBytes(32).toString('hex');
  const child = spawn(binary, ['--automation'], { stdio: ['pipe', 'pipe', 'pipe'] });
  let pending;
  let buffered = '';
  let stderr = '';
  const closed = new Promise(resolve => child.once('close', (code, signal) => {
    pending?.reject(new Error(`App closed (${code ?? signal})`));
    pending = undefined;
    resolve({ code, signal, stderr });
  }));
  child.stderr.setEncoding('utf8').on('data', text => { stderr = (stderr + text).slice(-4096); });
  child.stdin.on('error', error => pending?.reject(error));
  child.on('error', error => pending?.reject(error));
  child.stdout.setEncoding('utf8').on('data', text => {
    buffered += text;
    if (buffered.length > 8 * 1024 * 1024 + 1) {
      pending?.reject(new Error('Oversized native response'));
      child.kill();
      return;
    }
    let newline;
    while ((newline = buffered.indexOf('\n')) >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      const waiter = pending;
      pending = undefined;
      try { waiter?.resolve(JSON.parse(line)); }
      catch { waiter?.reject(new Error('Invalid native response')); }
    }
  });

  function exchange(line, timeout = 20000) {
    if (pending) throw new Error('Only one controller/request may act at a time');
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending = undefined;
        reject(new Error('Native response timeout'));
      }, timeout);
      pending = {
        resolve(value) { clearTimeout(timer); resolve(value); },
        reject(error) { clearTimeout(timer); pending = undefined; reject(error); },
      };
      child.stdin.write(line + '\n');
    });
  }

  const client = {
    child, token, closed,
    raw: exchange,
    request(command, credential = token) { return exchange(JSON.stringify({ token: credential, command })); },
    async stop() { child.kill(); return closed; },
  };
  try {
    client.hello = await exchange(JSON.stringify({ hello: hello ?? token }));
    return client;
  } catch (error) {
    await client.stop();
    throw error;
  }
}
