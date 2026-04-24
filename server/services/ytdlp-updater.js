// Auto-updater for yt-dlp.
//
// Maintains a project-local binary at <repo>/bin/yt-dlp that the server owns
// (no sudo, no system package conflict). On boot: downloads it if missing,
// then runs `yt-dlp -U` to self-update. Re-runs every UPDATE_INTERVAL_MS.

const fs = require('fs');
const path = require('path');
const https = require('https');
const { spawn } = require('child_process');
const log = require('./logger');

const BIN_DIR = process.env.YTDLP_BIN_DIR || path.resolve(__dirname, '../../bin');
const BIN_PATH = path.join(BIN_DIR, 'yt-dlp');
const DOWNLOAD_URL =
  process.env.YTDLP_DOWNLOAD_URL ||
  'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp';
const UPDATE_INTERVAL_MS =
  parseInt(process.env.YTDLP_UPDATE_INTERVAL_MS) || 6 * 60 * 60 * 1000;
const SPAWN_TIMEOUT_MS =
  parseInt(process.env.YTDLP_SPAWN_TIMEOUT_MS) || 120_000;

function download(url, dest, redirects = 0) {
  return new Promise((resolve, reject) => {
    if (redirects > 5) return reject(new Error('too many redirects'));
    const req = https.get(url, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        return resolve(download(res.headers.location, dest, redirects + 1));
      }
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
      }
      const tmp = dest + '.tmp';
      const file = fs.createWriteStream(tmp, { mode: 0o755 });
      res.pipe(file);
      file.on('finish', () => file.close(() => {
        fs.rename(tmp, dest, (err) => (err ? reject(err) : resolve()));
      }));
      file.on('error', (err) => { fs.unlink(tmp, () => reject(err)); });
    });
    req.on('error', reject);
    req.setTimeout(
      parseInt(process.env.YTDLP_DOWNLOAD_TIMEOUT_MS) || 60_000,
      () => req.destroy(new Error('download timeout')),
    );
  });
}

function run(bin, args, timeoutMs = SPAWN_TIMEOUT_MS) {
  return new Promise((resolve) => {
    const child = spawn(bin, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', (b) => (stdout += b.toString()));
    child.stderr.on('data', (b) => (stderr += b.toString()));
    const timer = setTimeout(() => child.kill('SIGKILL'), timeoutMs);
    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({ code, stdout: stdout.trim(), stderr: stderr.trim() });
    });
    child.on('error', (err) => {
      clearTimeout(timer);
      resolve({ code: -1, stdout: '', stderr: err.message });
    });
  });
}

async function ensureBinary() {
  if (!fs.existsSync(BIN_DIR)) fs.mkdirSync(BIN_DIR, { recursive: true });
  if (fs.existsSync(BIN_PATH)) return;
  log.info({ url: DOWNLOAD_URL, dest: BIN_PATH }, 'yt-dlp: bootstrapping binary');
  await download(DOWNLOAD_URL, BIN_PATH);
  fs.chmodSync(BIN_PATH, 0o755);
  log.info('yt-dlp: binary installed');
}

async function selfUpdate() {
  const before = (await run(BIN_PATH, ['--version'])).stdout;
  const upd = await run(BIN_PATH, ['-U', '--update-to', 'stable']);
  const after = (await run(BIN_PATH, ['--version'])).stdout;
  if (upd.code !== 0) {
    log.warn({ code: upd.code, stderr: upd.stderr }, 'yt-dlp: self-update failed');
    return;
  }
  if (before !== after) log.info({ before, after }, 'yt-dlp: updated');
  else log.info({ version: after }, 'yt-dlp: up-to-date');
}

async function tick() {
  try {
    await ensureBinary();
    await selfUpdate();
  } catch (err) {
    log.error({ err: err.message }, 'yt-dlp: update tick failed');
  }
}

let timer = null;
async function start() {
  await tick();
  timer = setInterval(tick, UPDATE_INTERVAL_MS);
  if (timer.unref) timer.unref();
}

function stop() {
  if (timer) { clearInterval(timer); timer = null; }
}

module.exports = { start, stop, BIN_PATH, ensureBinary, selfUpdate };
