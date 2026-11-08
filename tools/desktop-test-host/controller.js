import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { inspectReady, launchAutomation } from './client.js';

const [binary, outputArg] = process.argv.slice(2);
if (!binary || !outputArg) throw new Error('Usage: node controller.js TEST_HOST NEW_OUTPUT_DIR');
const output = resolve(outputArg);
await mkdir(output); // A fresh local directory; never overwrite earlier captures.
const app = await launchAutomation(resolve(binary));
let capture = 0;
const emit = value => process.stdout.write(JSON.stringify(value) + '\n');
try {
  await inspectReady(app);
  emit({ ready: true, pid: app.child.pid, protocol: app.hello.result?.protocol });
  for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    try {
      if (Buffer.byteLength(line) > 3000) throw new Error('Controller request too large');
      const command = JSON.parse(line);
      const response = await app.request(command);
      if (command === 'screenshot' && response.result?.data) {
        const path = resolve(output, `view-${++capture}.png`);
        await writeFile(path, Buffer.from(response.result.data, 'base64'));
        emit({ result: { screenshot: path } });
      } else {
        emit(response);
      }
      if (command === 'close' && response.result?.closing) {
        await app.closed;
        break;
      }
    } catch (error) {
      emit({ error: String(error) });
    }
  }
} finally {
  if (app.child.exitCode === null && app.child.signalCode === null) await app.stop();
}
