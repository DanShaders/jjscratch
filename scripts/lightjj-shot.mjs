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
// Each scene: { name, keys?: string[], settle?: ms, desc,
//               waitFor?: css-selector to await after keys,
//               act?: async (page) => {}  extra driving (clicks/typing),
//               requiresConflictFixture?: bool }.
// keys are dispatched in order with a small pause between each. View-switch
// keys (verified against lightjj App.svelte handleGlobalKeys):
//   "1" Revisions, "2" Branches/Bookmarks, "3" Merge,
//   "4" Oplog drawer (switches to log first), "5" Evolog drawer (needs a
//   selected revision — the working-copy @ is selected by default).
//   "t" toggles the theme.
// Cmd+K / Ctrl+K opens the command palette (handleGlobalOverrides, fires
// regardless of focus). The diff split toggle is a toolbar BUTTON
// (aria-label "Switch to split view", the ≡/◫ glyph) — there is no key for it.
// Navigation: "j"/"k" move the revision cursor. We always reset to view "1"
// at the top of every scene so scenes are independent and order-insensitive.
//
// `merge` runs ONLY against the conflict fixture (fixture-conflict/repo): the
// shared fixture has no conflicts, so lightjj's switchToMergeView (revset
// `conflicts() & mutable()`) finds nothing and bails back to the log. It is
// therefore excluded from the default run and only captured when explicitly
// selected with `--scene merge` (reference.sh points lightjj at the conflict
// fixture for that pass).
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
    waitFor: '.bp-root',
    settle: 600,
  },
  {
    name: 'diff-navigated',
    desc: 'Revisions view with cursor moved down a couple rows (press j j)',
    keys: ['1', 'j', 'j'],
    settle: 800,
  },
  {
    name: 'oplog',
    desc: 'Operation-log bottom drawer (press 4)',
    keys: ['4'],
    waitFor: '.oplog-panel',
    settle: 700,
  },
  {
    name: 'evolog',
    desc: 'Evolution-log drawer for the selected @ revision (press 5)',
    // @ (working copy) is selected on load and HAS evolog (snapshot + create),
    // so 5 opens a populated drawer. '@' re-selects it defensively first.
    keys: ['@', '5'],
    waitFor: '.evolog-panel',
    settle: 700,
  },
  {
    name: 'split-diff',
    desc: 'Diff panel toggled to side-by-side SPLIT view (Enter, then click ≡)',
    // Open @'s diff (Enter), then click the split toggle button. There is no
    // keybinding for split — only the toolbar button / Cmd+K palette command.
    keys: ['@', 'Enter'],
    settle: 400,
    async act(page) {
      await page.waitForSelector('.diff-panel, .diff-file', { timeout: 15000 });
      // The toolbar button toggles split/unified. Its aria-label reflects the
      // CURRENT state: "Switch to split view" => currently unified (click it);
      // "Switch to unified view" => already split (nothing to do). Idempotent so
      // a diff left split by a prior scene doesn't break this one.
      const toSplit = await page.$('button[aria-label="Switch to split view"]');
      if (toSplit) {
        await toSplit.click();
        await sleep(600);
      } else if (!(await page.$('button[aria-label="Switch to unified view"]'))) {
        throw new Error('diff split toggle button not found (diff not open?)');
      }
    },
  },
  {
    name: 'palette',
    desc: 'Cmd+K command palette overlay with a typed filter',
    keys: [],
    settle: 300,
    async act(page) {
      // Cmd/Ctrl+K. Meta is unreliable headless, so use Control (lightjj binds
      // both: `e.metaKey || e.ctrlKey`).
      await page.keyboard.down('Control');
      await page.keyboard.press('k');
      await page.keyboard.up('Control');
      await page.waitForSelector('.palette', { timeout: 10000 });
      await page.waitForSelector('.palette-input', { timeout: 10000 });
      // Type a query to capture the filtered state.
      await page.type('.palette-input', 'split');
      await sleep(500);
    },
  },
  {
    name: 'light',
    desc: 'Whole UI in light theme (press t)',
    keys: ['t'],
    settle: 700,
  },
  {
    name: 'merge',
    desc: 'Merge view: ConflictQueue + 3-pane (press 3, conflict fixture only)',
    keys: ['3'],
    waitFor: '.merge-panel',
    settle: 900,
    requiresConflictFixture: true,
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

// Send one key as a real user keystroke. Bare digits map to Digit codes;
// single printable punctuation (e.g. "@") is typed as a character (press()
// would need the physical key + shift); everything else (letters, "Enter",
// "Escape", …) is a puppeteer key name pressed directly.
async function sendKey(page, key) {
  await page.bringToFront();
  if (/^[0-9]$/.test(key)) {
    await page.keyboard.press(`Digit${key}`);
  } else if (key.length === 1 && !/[a-zA-Z]/.test(key)) {
    await page.keyboard.type(key);
  } else {
    await page.keyboard.press(key);
  }
  await sleep(180);
}

// Health probe: confirm we did not capture an all-white/blank frame. `.graph-row`
// only exists in the log view; non-log scenes (oplog/evolog/palette/merge) are
// validated by their own waitFor selector instead, so rows==0 is fine there.
async function notBlank(page) {
  return page.evaluate(() => {
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

    // `merge` only runs when explicitly selected (it needs the conflict fixture;
    // see SCENES comment). Default full runs skip it.
    let scenes = ONLY_SCENE
      ? SCENES.filter((s) => s.name === ONLY_SCENE)
      : SCENES.filter((s) => !s.requiresConflictFixture);
    if (scenes.length === 0) {
      throw new Error(`No scene matched --scene ${ONLY_SCENE}`);
    }

    for (const scene of scenes) {
      // Always start from a known state: Revisions view, no pending modes.
      // Two Escapes close any lingering overlay (palette) then mode; pressing
      // 1 returns to the log view and `t`-toggled themes carry over by design
      // (each scene that cares sets its own theme).
      await page.keyboard.press('Escape');
      await sleep(120);
      await page.keyboard.press('Escape');
      await sleep(120);
      await page.keyboard.press('Digit1');
      await sleep(250);

      for (const key of scene.keys) {
        await sendKey(page, key);
      }

      // Wait for the scene's view to actually mount before settling.
      if (scene.waitFor) {
        try {
          await page.waitForSelector(scene.waitFor, { timeout: 15000 });
        } catch {
          console.error(`[shot] ${scene.name}: waitFor "${scene.waitFor}" timed out`);
        }
      }

      // Extra driving (clicks / typing) that bare keys can't express.
      if (scene.act) await scene.act(page);

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

      // Per-scene cleanup so state doesn't bleed into later scenes (Escape +
      // Digit1 at the loop top resets the VIEW, but not sticky toggles):
      //   split-diff leaves config.splitView=true (a toolbar toggle, not view
      //     state) → click it back to unified.
      //   light leaves the theme light → press t back to dark.
      if (scene.name === 'split-diff') {
        const back = await page.$('button[aria-label="Switch to unified view"]');
        if (back) { await back.click(); await sleep(300); }
      }
      if (scene.name === 'light') await sendKey(page, 't');
    }
  } finally {
    await browser.close();
  }

  // Sanity gate: fail loudly if any image looks blank. Non-log scenes legitimately
  // have rows==0 (they replace the graph), so only gate rows for the log views.
  const ROW_REQUIRED = new Set(['revisions', 'diff-navigated', 'oplog', 'evolog', 'split-diff']);
  const blanks = results.filter(
    (r) => r.size < 5000 || r.textLen < 20 || (ROW_REQUIRED.has(r.scene) && r.rows === 0)
  );
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
