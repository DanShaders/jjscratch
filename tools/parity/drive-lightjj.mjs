#!/usr/bin/env node
// drive-lightjj.mjs — the lightjj side of the cross-driver INTERACTION harness.
//
// Replays a SHARED interaction script against the REAL lightjj web app via a
// headless Chrome (puppeteer-core + the puppeteer-cached Chrome 127), capturing
// a numbered PNG at each `shot` step. The same script file drives the native
// jjscratch `drive` binary, so the step-NN-<name>.png outputs line up for
// pixel-parity scoring.
//
// This is intentionally separate from scripts/lightjj-shot.mjs (the static
// reference capture) so neither breaks the other; both reuse the same Chrome
// discovery + viewport. lightjj itself is launched by the caller
// (scripts/compare-interaction.sh, which reuses scripts/reference.sh's env), and
// this script just connects to the already-serving URL.
//
// Usage:
//   node drive-lightjj.mjs --url http://localhost:3007 \
//        --script docs/parity/interaction/nav.txt --out <dir>
//
// Env overrides: LIGHTJJ_URL, LIGHTJJ_CHROME.
//
// Interaction-script format (one token per line; shared with drive.rs):
//   shot <name>   capture current frame -> <out>/step-NN-<name>.png
//   key  <k>      send a key (j/k/g/G/1/2/3, ArrowDown/ArrowUp/Home/End, ...)
//   # ...         comment; blank lines ignored

import { existsSync, mkdirSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '..', '..');

// puppeteer-core lives in the local refharness package (same as the reference
// harness). Resolve from there so cwd / NODE_PATH don't matter.
const HARNESS_DIR = join(REPO_ROOT, 'tools', 'refharness');
const require = createRequire(join(HARNESS_DIR, 'noop.cjs'));
const puppeteerEntry = require.resolve('puppeteer-core');
const puppeteer = (await import(pathToFileURL(puppeteerEntry).href)).default;

// ---- CLI / env -------------------------------------------------------------
function arg(flag, fallback) {
  const i = process.argv.indexOf(flag);
  return i !== -1 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const URL = arg('--url', process.env.LIGHTJJ_URL || 'http://localhost:3007');
const SCRIPT = arg('--script', null);
const OUT_DIR = arg('--out', join(REPO_ROOT, 'docs', 'parity', 'interaction', 'lightjj'));

if (!SCRIPT) {
  console.error('[drive] ERROR: --script <file> is required');
  process.exit(1);
}

// Match the reference harness viewport EXACTLY so outputs are comparable
// (1280x800 logical @ deviceScaleFactor 2 = 2560x1600, same as the jjscratch
// `drive` binary at --scale 2).
const VIEWPORT = { width: 1280, height: 800, deviceScaleFactor: 2 };

// ---- helpers ---------------------------------------------------------------
function findChrome() {
  if (process.env.LIGHTJJ_CHROME && existsSync(process.env.LIGHTJJ_CHROME)) {
    return process.env.LIGHTJJ_CHROME;
  }
  const base = join(homedir(), '.cache', 'puppeteer', 'chrome');
  if (!existsSync(base)) {
    throw new Error(`No cached Chrome dir at ${base} and $LIGHTJJ_CHROME unset`);
  }
  for (const entry of readdirSync(base)) {
    const exe = join(base, entry, 'chrome-linux64', 'chrome');
    if (existsSync(exe)) return exe;
  }
  throw new Error(`No chrome-linux64/chrome found under ${base}`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Map a shared script key token to a puppeteer key name. Bare digits become
// DigitN; single letters and spelled-out names (ArrowDown, Home, End) pass
// through. `key.press` already handles Shift internally for "G" via the literal.
function toPuppeteerKey(k) {
  if (/^[0-9]$/.test(k)) return `Digit${k}`;
  return k;
}

async function pressKey(page, k) {
  await page.bringToFront();
  // Uppercase letters (e.g. "G") must arrive shifted so the app sees "G".
  if (/^[A-Z]$/.test(k)) {
    await page.keyboard.down('Shift');
    await page.keyboard.press(`Key${k}`);
    await page.keyboard.up('Shift');
  } else {
    await page.keyboard.press(toPuppeteerKey(k));
  }
  await sleep(200); // let the SVG graph + async diff settle
}

// Parse the shared script into a list of {cmd, arg}.
function parseScript(text) {
  const steps = [];
  text.split('\n').forEach((raw, i) => {
    const line = raw.trim();
    if (!line || line.startsWith('#')) return;
    const sp = line.indexOf(' ');
    const cmd = sp === -1 ? line : line.slice(0, sp);
    const a = sp === -1 ? '' : line.slice(sp + 1).trim();
    if (cmd !== 'key' && cmd !== 'shot') {
      throw new Error(`line ${i + 1}: unknown command '${cmd}'`);
    }
    steps.push({ cmd, arg: a, lineno: i + 1 });
  });
  return steps;
}

// ---- main ------------------------------------------------------------------
async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  const steps = parseScript(readFileSync(SCRIPT, 'utf8'));
  const executablePath = findChrome();
  console.log(`[drive] chrome: ${executablePath}`);
  console.log(`[drive] url:    ${URL}`);
  console.log(`[drive] script: ${SCRIPT}`);
  console.log(`[drive] out:    ${OUT_DIR}`);

  const browser = await puppeteer.launch({
    executablePath,
    headless: 'new',
    args: [
      '--no-sandbox',
      '--headless=new',
      '--disable-gpu',
      '--disable-dev-shm-usage',
      `--window-size=${VIEWPORT.width},${VIEWPORT.height}`,
    ],
    defaultViewport: VIEWPORT,
  });

  let shotIdx = 0;
  const captured = [];
  try {
    const page = await browser.newPage();
    await page.setViewport(VIEWPORT);

    // Initial load: wait for the revision graph to render.
    await page.goto(URL, { waitUntil: 'networkidle2', timeout: 30000 });
    await page.waitForSelector('.revision-list', { timeout: 20000 });
    await page.waitForSelector('.graph-row', { timeout: 20000 });
    await sleep(800); // fonts, SVG graph, async diff load

    // Start from a known state: Revisions view (1), cursor on the working copy
    // (lightjj's default), no pending modes. The jjscratch driver seeds the
    // cursor on the working copy too, so both begin identically.
    await page.keyboard.press('Escape');
    await sleep(120);
    await page.keyboard.press('Digit1');
    await sleep(300);

    for (const step of steps) {
      if (step.cmd === 'key') {
        if (!step.arg) throw new Error(`line ${step.lineno}: 'key' needs an argument`);
        await pressKey(page, step.arg);
        console.log(`  key ${step.arg}`);
      } else {
        // shot
        const name = step.arg || 'frame';
        await sleep(250); // settle diff load before capturing
        const outPath = join(OUT_DIR, `step-${String(shotIdx).padStart(2, '0')}-${name}.png`);
        await page.screenshot({ path: outPath, fullPage: false });
        const size = statSync(outPath).size;
        console.log(`  shot #${String(shotIdx).padStart(2, '0')} ${name} -> ${outPath} (${size} bytes)`);
        captured.push({ name, path: outPath, size });
        shotIdx += 1;
      }
    }
  } finally {
    await browser.close();
  }

  if (shotIdx === 0) {
    console.error('[drive] ERROR: script produced no `shot` steps');
    process.exit(2);
  }
  const blanks = captured.filter((c) => c.size < 5000);
  if (blanks.length) {
    console.error('[drive] BLANK/SUSPICIOUS shots:', blanks.map((b) => b.name).join(', '));
    process.exitCode = 2;
  } else {
    console.log(`[drive] OK - captured ${shotIdx} step(s) into ${OUT_DIR}`);
  }
}

main().catch((err) => {
  console.error('[drive] FAILED:', err);
  process.exit(1);
});
