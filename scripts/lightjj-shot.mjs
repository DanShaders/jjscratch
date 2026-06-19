#!/usr/bin/env node
// Reference-image capture for the REAL lightjj app.
//
// Drives a headless Chrome (the puppeteer-cached Chrome 127) at lightjj via
// puppeteer-core and saves ground-truth PNGs that jjscratch is pixel-compared
// against. Scenes are data-driven (see SCENES below) so more can be added later.
//
// Usage:
//   node lightjj-shot.mjs [--url http://localhost:3007] [--out <dir>] [--scene name]
//
// Env overrides: LIGHTJJ_URL, REF_OUT_DIR, LIGHTJJ_CHROME.
//
// Resolution order for the Chrome executable:
//   1. $LIGHTJJ_CHROME
//   2. first match of ~/.cache/puppeteer/chrome/*/chrome-linux64/chrome
// We deliberately reuse the cached browser and never download a new one.

import { existsSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '..');

// puppeteer-core is installed in the local refharness package (tools/refharness),
// not next to this script. Resolve it from there so the script is runnable
// regardless of cwd / NODE_PATH (ESM ignores NODE_PATH).
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
const OUT_DIR = arg('--out', process.env.REF_OUT_DIR || join(REPO_ROOT, 'docs', 'reference'));
const ONLY_SCENE = arg('--scene', null); // optionally run a single scene by name

const VIEWPORT = { width: 1280, height: 800, deviceScaleFactor: 2 };

// ---- Scene definitions (data-driven) --------------------------------------
// Each scene: { name, keys?: string[], settle?: ms, desc }.
// keys are dispatched in order with a small pause between each. View-switch
// keys: "1" Revisions, "2" Branches/Bookmarks, "3" Merge, "4" Oplog, "5" Evolog.
// Navigation: "j"/"k" move the revision cursor. We always reset to view "1"
// at the top of every scene so scenes are independent and order-insensitive.
const SCENES = [
  {
    name: 'revisions',
    desc: 'Default revision graph view',
    keys: [],
    settle: 600,
  },
  {
    name: 'branches',
    desc: 'Bookmarks / Branches panel (press 2)',
    keys: ['2'],
    settle: 600,
  },
  {
    name: 'diff-navigated',
    desc: 'Revisions view with cursor moved down a couple rows (press j j)',
    keys: ['1', 'j', 'j'],
    settle: 800,
  },
];

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

async function pressKey(page, key) {
  // Dispatch to the focused document body the same way a user would type.
  await page.bringToFront();
  await page.keyboard.press(key);
  await sleep(180);
}

// Sample a few pixels of the screenshot buffer (decoded via the page) to make
// sure we did not capture an all-white/blank frame.
async function notBlank(page) {
  return page.evaluate(() => {
    // Cheap heuristic: the app background is a dark/themed surface, and the
    // body should contain the toolbar + revision list with real text.
    const body = document.body;
    const txt = (body.innerText || '').replace(/\s+/g, '');
    const hasRows = document.querySelectorAll('.graph-row').length;
    const bg = getComputedStyle(body).backgroundColor;
    return { textLen: txt.length, rows: hasRows, bg };
  });
}

// ---- main ------------------------------------------------------------------
async function main() {
  mkdirSync(OUT_DIR, { recursive: true });
  const executablePath = findChrome();
  console.log(`[shot] chrome:   ${executablePath}`);
  console.log(`[shot] url:      ${URL}`);
  console.log(`[shot] out:      ${OUT_DIR}`);

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

  const results = [];
  try {
    const page = await browser.newPage();
    await page.setViewport(VIEWPORT);

    // Initial load: wait for the revision graph to render.
    await page.goto(URL, { waitUntil: 'networkidle2', timeout: 30000 });
    await page.waitForSelector('.revision-list', { timeout: 20000 });
    await page.waitForSelector('.graph-row', { timeout: 20000 });
    await sleep(800); // settle: fonts, SVG graph, async diff load

    const scenes = ONLY_SCENE ? SCENES.filter((s) => s.name === ONLY_SCENE) : SCENES;
    if (scenes.length === 0) {
      throw new Error(`No scene matched --scene ${ONLY_SCENE}`);
    }

    for (const scene of scenes) {
      // Always start from a known state: Revisions view, no pending modes.
      await page.keyboard.press('Escape');
      await sleep(120);
      await page.keyboard.press('Digit1');
      await sleep(250);

      for (const key of scene.keys) {
        // Map bare digit/letter strings to puppeteer key names.
        const keyName = /^[0-9]$/.test(key) ? `Digit${key}` : key;
        await pressKey(page, keyName);
      }
      await sleep(scene.settle ?? 500);

      const health = await notBlank(page);
      const outPath = join(OUT_DIR, `${scene.name}.png`);
      await page.screenshot({ path: outPath, fullPage: false });
      const size = statSync(outPath).size;
      console.log(
        `[shot] ${scene.name.padEnd(16)} -> ${outPath} (${size} bytes, ` +
          `rows=${health.rows}, textLen=${health.textLen}, bg=${health.bg})`
      );
      results.push({ scene: scene.name, path: outPath, size, ...health });
    }
  } finally {
    await browser.close();
  }

  // Sanity gate: fail loudly if any image looks blank.
  const blanks = results.filter((r) => r.size < 5000 || r.rows === 0 || r.textLen < 20);
  if (blanks.length) {
    console.error('[shot] BLANK/SUSPICIOUS scenes:', blanks.map((b) => b.scene).join(', '));
    process.exitCode = 2;
  } else {
    console.log(`[shot] OK - ${results.length} reference image(s) captured.`);
  }
}

main().catch((err) => {
  console.error('[shot] FAILED:', err);
  process.exit(1);
});
