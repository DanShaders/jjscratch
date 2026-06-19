# lightjj UI Specification (for Vello/native reimplementation)

This document specifies the visual/layout system of **lightjj** (a Svelte web GUI for
Jujutsu) precisely enough to reproduce pixel-for-pixel in a native renderer that has
never seen the app.

## Conventions & source of truth

- The **DEFAULT theme is DARK.** In `frontend/src/theme.css` the primaries are defined
  on `:root, :root[data-theme="dark"]` (identical block). `themes.ts` lists `dark`
  (`'Default Dark'`) first and `isThemeDark()` falls back to `true`. A "Default Light"
  theme also exists (`:root[data-theme="light"]`) and is the only built-in light theme.
- Colors live as CSS custom properties. There are **~32 per-theme PRIMARIES** and
  **~50 DERIVED vars** computed once in a theme-agnostic `:root` block via
  `color-mix(in srgb, X p%, Y)`. Below, every derived var is resolved to an approximate
  hex for the **dark** theme, with the formula shown.
- `color-mix(in srgb, A p%, transparent)` = color A at alpha `p/100` over whatever is
  behind it (these are rendered as semi-transparent fills, NOT pre-blended — keep them
  translucent in the renderer; the "resolved hex over --base" value is given only as a
  visual reference). `color-mix(in srgb, A p%, B)` with an opaque B = a solid blend.
- Box model is `border-box` everywhere. Body has `overflow:hidden` (no document scroll;
  panels scroll internally).
- `--anim-duration` = 150ms, `--anim-ease` = `cubic-bezier(0.2,0,0.2,1)`. Reduced-motion
  collapses duration to 0. For a static renderer these only matter for hover/active
  state transitions.

---

## 1. Color palette (DEFAULT = dark)

### 1.1 Primaries (dark theme, `:root` / `[data-theme="dark"]`)

| Var | Hex | Role |
|---|---|---|
| `--base` | `#0f0f13` | app background / page bg |
| `--mantle` | `#0f0f13` | panel headers, toolbars, secondary surfaces (== base here) |
| `--crust` | `#0a0a0e` | toolbar bg, statusbar bg (darkest) |
| `--surface2` | `#4e4e58` | gray-ramp darkest step — **borders/dividers only**, never text |
| `--overlay0` | `#8a8a94` | secondary glyphs, commit-id, timestamps, placeholders |
| `--overlay1` | `#9a9aa4` | hover variant of overlay0 |
| `--subtext0` | `#8a8a94` | default button/badge text, panel sub-labels |
| `--subtext1` | `#e2e2e6` | panel title text (== text here) |
| `--text` | `#e2e2e6` | primary foreground text |
| `--amber` (accent) | `#ffa726` | THE accent: selection, working-copy, change-id, primary buttons, active nav |
| `--green` | `#66bb6a` | additions, success, checked rows |
| `--red` | `#ef5350` | deletions, danger, conflicts |
| `--blue` | `#6880b8` | links, question-severity, syn fallback |
| `--mauve` | `#c792ea` | keywords (syntax) |
| `--lavender` | `#b4befe` | accent variant (rarely used directly) |

### 1.2 Graph lane colors (`--graph-0..7`) — dark

Decorative, muted (Tier 3). Rendered at **0.45 opacity for lines, 0.8 for nodes**.

| Var | Hex | | Var | Hex |
|---|---|---|---|---|
| `--graph-0` | `#BF8A30` (gold) | | `--graph-4` | `#6880B8` (blue) |
| `--graph-1` | `#B86848` (rust) | | `--graph-5` | `#5098A0` (teal) |
| `--graph-2` | `#A8506A` (mauve-red) | | `--graph-6` | `#5AA058` (green) |
| `--graph-3` | `#8868A8` (purple) | | `--graph-7` | `#A89838` (olive) |

(Light theme equivalents are darker: `#9A6E18,#984830,#883858,#6A4890,#4860A0,#387880,#3A8038,#887820`.)

### 1.3 Syntax token colors (`--syn-*`) — dark

| Token class | Var | Hex | Notes |
|---|---|---|---|
| `.tok-keyword` | `--syn-keyword` | `#c792ea` | |
| `.tok-string`, `.tok-string2` | `--syn-string` | `#a3be8c` | |
| `.tok-number`, `.tok-integer` | `--syn-number` | `#f78c6c` | |
| `.tok-comment`, `.tok-meta` | `--syn-comment` | `#6c7086` | comment is *italic* |
| `.tok-typeName`, `.tok-className` | `--syn-type` | `#f9e2af` | |
| `.tok-propertyName`, `.tok-definition` | `--syn-property` | `#89b4fa` | |
| `.tok-operator` | `--syn-operator` | `#9399b2` | |
| `.tok-punctuation` | `--syn-punct` | `#6c7086` | |
| `.tok-atom/.bool/.null/.literal` | `--syn-atom` | `#f78c6c` | |
| (variableName / identifiers) | — | inherits `--text` (`#e2e2e6`) | deliberately unstyled |

### 1.4 Derived vars (resolved for dark)

Format: `name = formula → approx hex (effective over base when relevant)`.

- `--surface0 = mix(text 4%, transparent)` → translucent `#e2e2e6` @ 4% → over base ≈ `#171719`. Hover bg, kbd bg, file-tab hover.
- `--surface1 = mix(text 7%, transparent)` → `#e2e2e6` @ 7% → over base ≈ `#1e1e21`. Default borders.
- `--text-faint = mix(text 45%, transparent)` → `#e2e2e6` @ 45% → over base ≈ `#71717a`. Timestamps, line numbers, placeholders, dimmed id tails. **The only correct dim-text color.**
- `--bg-hover = --surface0` (≈ `#171719`).
- `--bg-selected = mix(amber 8%, transparent)` → amber @ 8% → over base ≈ `#241c19`.
- `--bg-checked = mix(green 8%, transparent)` → green @ 8% → ≈ `#16221a`.
- `--bg-checked-selected = mix(green 12%, transparent)` → ≈ `#1a2a1d`.
- `--bg-error = mix(red 10%, transparent)` → ≈ `#26181a`.
- `--bg-warning = mix(amber 10%, transparent)` → ≈ `#28201a`.
- `--bg-success = mix(green 8%, transparent)` → ≈ `#16221a`.
- `--bg-active = mix(amber 14%, transparent)` → ≈ `#2c211a` (active seg-btn).
- `--bg-btn-primary-hover = mix(amber 85%, text)` → `#ffa726`·0.85 + `#e2e2e6`·0.15 ≈ `#f0ad44`.
- `--bg-hunk-header = mix(text 3%, transparent)` → ≈ `#15151a`.
- `--border-hunk-header = mix(text 5%, transparent)` → ≈ `#1a1a1e`.
- `--bg-diff-empty = mix(text 2%, transparent)` → ≈ `#131317`.
- `--bg-diff-header-hover = mix(text 3%, base)` (opaque) ≈ `#141418`.
- `--bg-bookmark = --surface0`; `--border-bookmark = --surface1`.
- `--badge-workspace-bg = --surface0`; `--border-workspace = --surface1`.
- `--bg-pr = --surface0`; `--border-pr = --surface1`; `--border-pr-hover = --surface2`.

**Diff fills (semi-transparent — keep translucent):**
- `--diff-add-bg = mix(green 10%, transparent)` → green @ 10% (over base ≈ `#18231b`).
- `--diff-remove-bg = mix(red 10%, transparent)` → red @ 10% (over base ≈ `#251a1b`).
- `--diff-add-word = mix(green 22%, transparent)` → green @ 22% (word-level add highlight).
- `--diff-remove-word = mix(red 22%, transparent)` → red @ 22% (word-level remove highlight).

**Status/type badges (fills):**
- `--badge-add-bg = mix(green 12%, transparent)`, text `--green`.
- `--badge-modify-bg = mix(amber 12%, transparent)`, text `--amber`.
- `--badge-delete-bg = mix(red 12%, transparent)`, text `--red`.
- `--badge-other-bg = mix(amber 12%, transparent)`, text `--amber` (rename/source/target).

**Conflict region colors:**
- `--conflict-boundary-border = mix(red 20%, transparent)`; `--conflict-boundary-bg = mix(red 6%, transparent)`.
- `--conflict-side1-border = --red` (`#ef5350`); `--conflict-side1-bg = mix(red 6%, transparent)`.
- `--conflict-side2-border = mix(red 50%, transparent)`; `--conflict-side2-bg = mix(red 3%, transparent)`.

**Search highlight:**
- `--search-match-bg = mix(amber 20%, transparent)`; `--search-match-current-bg = mix(amber 45%, transparent)` + 1px amber outline.

**Scrollbars:** track transparent; thumb `mix(text 12%, transparent)` ≈ `#2a2a2e`. Global
scrollbar 6px wide/tall, radius 3px; panel-content scrollbars overridden to 8px,
thumb `--surface0`, hover `--surface1`.

**Misc:** `::selection` = `mix(amber 25%, transparent)`. `--backdrop` (modal) = `rgba(0,0,0,0.5)`
(light: 0.3). `--shadow-heavy` = `0 20px 60px rgba(0,0,0,0.3)` (light: 0.15).

---

## 2. Typography

### 2.1 Font families (config-overridable stacks)

- `--font-ui` = `'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif` — all chrome/UI text.
- `--font-mono` = `'JetBrains Mono', 'SF Mono', 'Fira Code', monospace` — change/commit ids, diff lines, kbd, hunk headers, workspace selector.
- Markdown prose (not chrome): `--font-md-body` = `system-ui, -apple-system, sans-serif`; heading/display fall back to body; `--font-md-code` = `--font-mono`.

### 2.2 Size scale

`--font-size` (config base, default **14px**) drives a set of additive-offset derived
sizes. Floors via `max()` on the small tiers. At the default 14px base:

| Var | Formula | px @14 base |
|---|---|---|
| `--fs-3xs` | `max(8, base-5)` | **9** |
| `--fs-2xs` | `max(9, base-4)` | **10** |
| `--fs-xs` | `max(10, base-3)` | **11** |
| `--fs-sm` | `base-2` | **12** |
| `--fs-md` | `base-1` | **13** |
| (base) | `base` | **14** |
| `--fs-lg` | `base+1` | **15** |
| `--fs-xl` | `base+3` | **17** |

(The code comments say "13" base in places — that is stale; `FONT_SIZE_DEFAULT = 14`.
Configurable range clamps; small tiers floor at 8/9/10/11.)

### 2.3 Line heights & weights (chrome)

- Graph rows: fixed `height:18px`, `line-height:18px`, `font-size:--fs-md (13)`.
- Diff lines: `line-height:18px` (literal, must match CodeMirror), `font-size:--fs-md (13)`, mono.
- Buttons `.btn`: `line-height:1.4`, `--fs-sm (12)`.
- Panel title: `font-weight:700`, uppercase, `letter-spacing:0.05em`, `--fs-sm`.
- Change-id (graph): mono, `--fs-sm`, `font-weight:600`, `letter-spacing:0.02em`, color amber.
- Weights used: 400 (normal), 500, 600 (badges/ids), 700 (titles/alerts), 800 (mode badge / side-badge).
- Prose: base `max(15.5, base+2.5)`, `line-height:1.68`, weight 370; h1 2.6em/760, h2 1.7em/720, etc. (Prose is only relevant for the Markdown preview surface.)

---

## 3. Layout

Top-level vertical stack (`.app`, `height:100vh`, column, overflow hidden):

```
┌──────────────────────────────────────────────────────────┐
│ .toolbar            height 34px   bg --crust   ─border-b  │
├──────────────────────────────────────────────────────────┤
│ .tab-bar            height 26px   bg --base    ─border-b  │  (rendered by App via snippet)
├──────────────────────────────────────────────────────────┤
│ .workspace  flex:1  row                                   │
│  ┌─────────────────────────┬──┬──────────────────────────┐│
│  │ .revision-panel-wrapper │░░│  diff panel (flex:1)      ││
│  │  width 420px (default)  │  │                           ││
│  │  min 280, max 600       │  │                           ││
│  │  (revset bar + graph)   │##│  panel-divider 4px        ││
│  └─────────────────────────┴──┴──────────────────────────┘│
├──────────────────────────────────────────────────────────┤
│ .statusbar          height 24px   bg --crust   ─border-t  │  (StatusBar / footer)
└──────────────────────────────────────────────────────────┘
```

(Plus fixed overlays: MessageBar, modals, file-history overlay, optional bottom
Oplog/Evolog drawers above the statusbar.)

### 3.1 Top toolbar (`.toolbar`)
- `height:34px`, `padding:0 10px`, `bg --crust` (#0a0a0e), `border-bottom:1px solid --surface1`, `gap:8px`, `align-items:center`, `justify-content:space-between`, `user-select:none`.
- **Left group** (`.toolbar-left`, `gap:8px`, `flex:1`, min-width:0):
  1. **Logo**: 16×16 svg (`/logo.svg` dark, `/logo-light.svg` light). When update available, a `.update-dot`: 9×9 circle, `bg --red`, `1.5px --crust` border, top -3 right -4, plus a faint red ring shadow.
  2. `.toolbar-divider`: 1px wide × 14px tall, `bg --surface1`. Used to separate clusters.
  3. **Workspace selector** (`.toolbar-ws-btn`, only if a workspace exists): `padding:3px 8px`, `border:1px solid --surface1`, `radius:4px`, mono, `--fs-sm`, color `--subtext0`. Contents: glyph `◇` (color --subtext0, --fs-xs) + name (color --text) + chevron `▾`/`▴` (--fs-2xs, --text-faint) when >1 workspace. Hover: bg --bg-hover, border --surface2. Dropdown: abspos, `min-width:160px`, bg --mantle, 1px --surface1 border, radius 5px, padding 3px, shadow-heavy; options `padding:5px 8px` radius 4 mono --fs-sm; active option amber; a separator `.toolbar-ws-sep` 1px, margin 3px 4px, bg --surface1.
  4. Another `.toolbar-divider`.
  5. **Nav tabs** = a `.seg` segmented control (see §6.2) containing three `.seg-btn`:
     - `◉ Revisions ` + `<kbd class="nav-hint">1</kbd>`
     - `⑂ Branches ` + `<kbd>2</kbd>`
     - `⧉ Merge ` + `<kbd>3</kbd>`
     - (optionally a 4th active `▤ {filename}` for doc mode)
     Active button: `bg --bg-active`, color `--amber`, weight 600.
  6. `.toolbar-divider`.
  7. **Drawer toggles** (`.toolbar-nav-btn`, NOT segmented — borderless): `padding:3px 8px`, no border, radius 4, color --subtext0, --fs-sm, line-height 1.4, nowrap.
     - `⟲ Oplog ` + `<kbd>4</kbd>`
     - `◐ Evolog ` + `<kbd>5</kbd>`
     Hover (inactive): bg --bg-hover, color --text. Active (`.toolbar-nav-active`): color --amber, weight 600, and its nav-hint kbd gets amber text + `border-color mix(amber 30%,transparent)`.
  8. `.toolbar-divider`.
  9. **Search button** (`.toolbar-search`): `padding:3px 8px`, `bg --surface0`, no border, radius 4, color --text-faint, --fs-sm. Text "Search…" + a kbd `⌘K`/`Ctrl+K` (`.toolbar-search-kbd`: mono, --fs-2xs, --text-faint, 1px --surface1 border, padding 0 4px, radius 3). Hover: color --subtext0.
- **Right group** (`.toolbar-right`, gap 8): theme toggle `.btn.toolbar-theme` (no border, --font-size, padding 3px 6px) showing `☀` (dark mode → switch to light) or `●` (light mode).

### 3.2 Tab bar (`.tab-bar`)
- `height:26px`, `bg --base`, `border-bottom:1px solid --surface1`, `padding-left:10px`, mono, --fs-sm, align-items stretch.
- `.tab`: `gap:6px`, `padding:0 10px`, transparent bg, `border-bottom:2px solid transparent`, `margin-bottom:-1px` (overlaps bar border), color --subtext0, max-width 200px. Glyph `▪` (--fs-3xs, opacity 0.5). Hover (inactive): bg --bg-hover, color --text. **Active**: color --text, `border-bottom-color:--amber`, glyph color amber/opacity 1.
- `.tab-close` `×`: hidden (opacity 0) until tab hover/active (0.5), hover full + bg --surface1. font --font-size, margin-right -4.
- `.tab-new` `+`: 26px wide, color --text-faint, --fs-lg; hover bg --bg-hover color --text. Or, while opening, `.tab-path-input`: 18px tall, 260px wide, bg --surface0, 1px --surface1 border, radius 3; focus border amber.

### 3.3 Two-column workspace
- `.workspace`: flex row, flex:1.
- `.revision-panel-wrapper`: `flex-shrink:0`, **`min-width:280px`, `max-width:600px`**, inline `width:{config.revisionPanelWidth}px` (**default 420**). Column: revset filter bar (+ preset chips) on top, then the RevisionGraph panel.
- `.panel-divider`: 4px wide, `cursor:col-resize`, z-index 1. Visual line via `::after`: 1px at left:1px, transparent → `--surface2` on hover/drag. Drag clamps width to [280,600] from clientX.
- Diff panel: `.diff-panel` `flex:1; min-width:0`.
- Merge & doc views **hide the revision panel entirely** and go full width.

### 3.4 Revset filter bar (top of revision panel)
- `.revset-filter-bar`: `padding:4px 8px`, `gap:6px`, `bg --mantle`, `border-bottom:1px solid --surface0`.
- `.revset-icon` `$`: --text-faint, --fs-md, weight 700.
- `.revset-input`: flex:1, `bg --base`, `1px solid --surface1`, radius 3, `padding:3px 6px`, --fs-md; focus border amber; placeholder color --surface1.
- Below it, optional `.preset-chips` row: `padding:0 8px 4px`, gap 4, wrap; each `.preset-chip` --fs-sm, `padding:2px 8px`, radius 3, bg --surface0, color --subtext0, transparent border; hover bg --surface1; **active**: border amber, color amber.

### 3.5 Status bar (`.statusbar` / footer)
- `height:24px`, `padding:0 10px`, `bg --crust`, `border-top:1px solid --surface1`, --fs-sm, color --subtext0, user-select none, justify space-between.
- When an inline **mode** is active the top-border tints: rebase/split → amber, squash → green. (See §6.5 for the mode badge + key hints.)
- Idle: left side shows `statusText` (e.g. `12 revisions | @ abcdefgh | 2 conflicts`) OR per-view key hints (`.key-hints`, color --text-faint, --fs-xs, each `<kbd>` bg --surface0 padding 0 3 radius 2 no border).

### 3.6 Panel chrome (shared `.panel-header` / `.panel-title`)
- `.panel-header`: `height:34px`, `padding:0 12px`, `bg --mantle`, `border-bottom:1px solid --surface0`, flex space-between, align center, user-select none.
- `.panel-title`: --fs-sm, weight 700, UPPERCASE, letter-spacing 0.05em, color --subtext1.
- RevisionGraph header additionally shows `<kbd class="nav-hint">j</kbd><kbd>k</kbd>` after the title, an optional `.view-btn-active` view-label pill, and a right-aligned `.panel-badge` (bg --surface0, color --subtext0, padding 0 6, radius 8, --fs-xs, weight 600) with the revision count.

---

## 4. Revision graph rows

The graph is a list of **flattened lines**, each a `.graph-row` of **fixed `height:18px`,
`line-height:18px`, `font-size:--fs-md(13)`, overflow:hidden, position:relative**. Every
revision expands into: 1 node line, an optional bookmark line, and a description/meta
line — all 18px, all sharing the same `data-entry` index. This fixed height is load-bearing
for graph-pipe continuity; nothing may stretch it.

Row left structure: `.check-gutter` (14px wide, text-align center, color --green @0.85
opacity, --fs-xs, padding-left 2) then the SVG gutter, then content.

### 4.1 Gutter SVG geometry (GraphSvg)
Constants: **CELL_W = 10px** per character column, **ROW_H = 18**, node radius **NODE_R = 4**,
working-copy radius **WC_R = 5**, **LINE_W = 2** (stroke), 8 graph colors. Lane = `floor(col/2)`,
so each lane is 20px wide; node center x = `col*10 + 5`; cy = 9.
SVG width = `gutterWidth * 10` where gutterWidth = `min(maxGutterLen, 12)` columns. Lane
color = `palette[lane % 8]` = `--graph-{lane%8}`.

Opacities: **lines 0.45**, **nodes 0.8**, elided/`~` lines 0.3, hidden node 0.35.
`<line>` elements use `shape-rendering:crispEdges`; curves/circles stay anti-aliased.
`NODE_GAP = 7` — vertical lane segments stop 7px short of cy so hollow nodes have a clean gap.

Character → drawing:
- `│` vertical: full-height line, lane color, width 2, opacity 0.45.
- `~` elided: vertical dashed (`dasharray 2 3`), opacity 0.3.
- `─` horizontal: line across the cell at cy, opacity 0.45 (lane = max lane of its run).
- `├` / `┤`: vertical trunk (cell lane) + half-cell horizontal stub toward lane±1.
- `╮ ╯ ╭ ╰`: quadratic-curve elbows (Q through (x,cy)), round linecap, width 2, opacity 0.45.
- Node chars (`@ ○ ◆ × ◌`): two vertical lane segments (0→cy-7 and cy+7→18) at 0.45, then the node glyph:
  - **`@` working copy**: amber **concentric circle** — outer ring `r = WC_R+1 = 6`, `fill:none`, `stroke:--amber`, `stroke-width:1.8`; inner filled dot `r = 2.5`, `fill:--amber`.
  - **`◆` immutable**: a 7×7 `<rect>` (`x=cx-3.5,y=cy-3.5`) `rx=1`, `fill = lane color`, **`opacity:0.5`** (dimmer), rotated 45° → diamond.
  - **`×` conflict**: filled circle `r = WC_R(5)`, `fill:--red` (full opacity, no graph dimming), with two `--base`-colored diagonal strokes (width 1.5) forming an ✕.
  - **`◌` hidden**: hollow circle `r = NODE_R-0.5 = 3.5`, `stroke = lane color`, width 1.2, opacity 0.35.
  - **`○` normal/mutable**: filled circle `r = NODE_R(4)`, `fill = lane color`, opacity 0.8.
  - If the entry is **divergent**, an extra dashed ring (`r = NODE_R+3 = 7`, stroke lane color, width 1, `dasharray 2 2`, opacity 0.5) is drawn around any node.

### 4.2 Node line content (`.node-line-content`, inline-flex, gap 6, baseline)
Right of the gutter on the node row: optional role badges (rebase/squash/split source/target
`<< label >>`, `.badge-source/.badge-target`: bg --badge-other-bg, color amber, 1px amber
border, --fs-xs, weight 700, radius 3), then `divergent`/`conflict` **alert badges**
(`.alert-badge`: bg --bg-error, color --red, 1px red border, --fs-xs, weight 700, radius 3),
then the description text: `.description-text` color --text --fs-md, or `.desc-placeholder`
"(no description)" / `.empty-label` "(empty)" in `--overlay0` --fs-md.

### 4.3 Bookmark line (`.bookmark-line-content`, gap 4)
Badges (all --fs-xs unless noted, line-height 1.15, radius 3, letter-spacing 0.02em):
- **Workspace badge** `◇ {ws}@`: bg --badge-workspace-bg(=surface0), color --subtext0, weight 600, 1px --border-workspace border. Current workspace: `opacity:0.55`, non-interactive. Others: clickable, hover border `--accent` color --text.
- **Local bookmark** `.bookmark-badge` `⑂ {name}`: bg --bg-bookmark, color --subtext0, weight 600, 1px --border-bookmark border, padding 0 5. Hover border --surface2. **Lane-tinted** when a node lane is known and not conflicted: border `mix(laneColor 50%,transparent)`, text = lane color. Conflicted: append red `??` (`.conflict-marker`); unsynced: append amber `*` (`.sync-marker`).
- **PR badge** `.pr-badge` `↗ {name} #{n}`: same chrome as bookmark, gap 3; draft → dashed border + opacity 0.75; `.pr-number` color --overlay0 weight 400.
- **Remote bookmark** `.remote-bookmark-badge` `{remote}/{name}`: --fs-2xs, weight 500, color --overlay0, transparent bg, 1px --surface0 border.

### 4.4 Description / meta line (`.desc-line-content`)
`.meta-line` (inline-flex, gap 8, baseline) holds:
- **Change id** `.change-id`: mono, --fs-sm, color **--amber**, weight 600, ls 0.02em.
  Rendered as `change_id.slice(0, change_prefix)` (highlighted amber) + `.id-rest`
  = `slice(change_prefix, 12)` in `--text-faint` weight 400. Divergent appends
  `.div-offset` `/N` in **--red weight 700**.
- **Commit id** `.commit-id`: mono, --fs-xs, color **--overlay0**, ls 0.02em — same
  prefix-highlight split, tail `.id-rest` in --text-faint.
- **author chip** (only if not mine): bg --surface0, color --subtext0, padding 0 5, --fs-xs, radius 3.
- **timestamp chip**: color --overlay0, --fs-xs.
In an inline mode, the operative row's desc line instead shows `.rebase-preview` (the jj
command), color --overlay0, --fs-md, italic.

### 4.5 Row states
- `.hovered:not(.selected)` (JS-tracked, whole revision group): `bg --bg-hover`.
- `.selected`: `bg rgba(amber / 0.04)` + **`box-shadow: inset 2px 0 0 --amber`** (2px amber left bar).
- `.checked`: `bg --bg-checked` (green 8%). `.checked.selected`: bg --bg-checked-selected + amber inset bar.
- `.implied` (gap-filled in multi-select): `bg rgba(green / 0.04)`; check-gutter shows `◌` in --overlay0.
- `.hidden-rev`: whole row `opacity:0.45`.
- `.immutable .description-text`: color **--overlay0** (dimmed) — this is the "immutable dimming."
- Check gutter shows `✓` (green) when checked, `◌` (overlay0) when implied.

### 4.6 Other graph elements
- **Batch-actions bar** (when checks active): `padding:4px 8px`, bg --bg-checked, border-bottom --border-bookmark; `.batch-label` green weight 600; `.btn` / `.btn-danger` actions.
- **Refresh bar**: always-mounted 2px strip; when refreshing, bg --surface0 with an amber indeterminate sweep; the list dims to `opacity:0.55`.
- **Elided marker**: `(elided revisions)` label --text-faint --fs-xs between two 1px dashed --surface1 rules.

---

## 5. Diff panel

Vertical stack inside `.diff-panel`: panel-header (or RevisionHeader snippet) → optional
file-list bar → optional annotations bar → diff toolbar → optional search bar → scrolling
`.panel-content` of `.diff-file` blocks.

### 5.1 Revision header (RevisionHeader, the single-target header slot)
`.revision-detail`: `padding:8px 12px`, bg --mantle, border-bottom --surface0, --fs-sm,
column, gap 4. `.detail-change-id`: mono, **amber**, weight 600, --fs-md (8-char slice).
`.detail-description-inline`: color --text, --fs-md, pre-wrap (collapsible to 1 line).
Actions: `.btn` "Describe", optional `.btn.btn-danger` "Divergence", a chevron expand btn.
Bookmark/PR badges identical chrome to §4.3 (`.detail-bookmark-badge` / `.detail-pr-badge`).

### 5.2 File-list bar (`.file-list-bar`)
`padding:6px 12px`, gap 8, bg --mantle, border-bottom --surface0. `.file-list-label`
"Files [ ] (N)" UPPERCASE --fs-xs weight 600 color --text-faint (conflict count in red).
`.total-stats`: `+N` green / `-N` red (--fs-sm weight 600). `.file-tabs` wrap, max-height 52px:
each `.file-tab` padding 4px 10px, --fs-sm, border-bottom 2px transparent; active → color --text
+ amber bottom-border + bold name. `.file-dot` 5px circle colored by type (A green, M amber,
D/C red, else --subtext0). Overflow indicator `▾` chip.

### 5.3 Diff toolbar (`.diff-toolbar`)
`padding:4px 12px`, bg --mantle, border-bottom --surface0, justify space-between. Left:
collapse/expand `.btn.btn-sm` (`⊞`/`⊟`) + an `.ann-hint` (`Alt`+click annotate · mod hover
definition, kbds via .nav-hint). Right: **unified/split toggle** `.btn.btn-sm` showing
`≡` (unified) or `◫` (split). Search bar (when open): `.search-input` bg --base, 1px
--surface1, mono --fs-md, focus border amber; `.search-count` --fs-sm --subtext0.

### 5.4 Per-file block (`.diff-file`)
`border-bottom:1px solid --surface0`, `content-visibility:auto`.
- **File header** `.diff-file-header`: `padding:7px 12px`, gap 8, bg --mantle, color --text,
  weight 600, --fs-md, border-bottom --surface0, **sticky top:0**, cursor pointer; hover bg
  --bg-diff-header-hover. Contains: collapse chevron `.collapse-icon` (--text-faint, rotates
  90°→0° collapsed), a **file-type badge** (`.file-type-badge` --fs-xs weight 700 radius 3:
  `badge-A` green-on-add-bg, `badge-M` amber-on-modify-bg, `badge-D` red-on-delete-bg,
  `badge-R` amber-on-other-bg), the path (`.file-dir` --text-faint weight 400 + `.file-name`
  --text weight 700), and `.file-stats` (`+N` `.stat-add` green / `-N` `.stat-del` red, --fs-sm weight 600).
  Reviewed file: `box-shadow: inset 3px 0 0 --green`.

### 5.5 Hunk header & expand
`.diff-hunk-header`: `padding:3px 12px`, gap 8, bg --bg-hunk-header, color --overlay0,
--fs-sm, mono, border-bottom --border-hunk-header. `.hunk-range` color --text-faint --fs-xs;
`.hunk-context` color --subtext0 italic. Hunk cursor (review): `box-shadow: inset 2px 0 0
amber, inset 0 0 0 1px amber` + bg --bg-selected. `.expand-btn` (context expansion): full
width, transparent, color --text-faint, dashed top/bottom borders (--border-hunk-header),
--fs-sm; `.expand-dots` letter-spacing 2px → amber on hover.

### 5.6 Diff lines (`.diff-lines` / `.diff-line`) — UNIFIED
`.diff-lines`: mono, --fs-md(13), **line-height:18px**.
`.diff-line`: hanging indent via `--diff-gutter-w:11ch`, `padding:0 12px 0 11ch`,
`text-indent:-11ch`, `white-space:pre-wrap`, `word-break:break-all`, `border-left:3px solid
transparent`, `tab-size:4`, `position:relative`.
- **Line-number gutter** `.line-num`: inline-block, `min-width:4ch`, right-aligned,
  `padding-right:1ch`, `margin-right:1ch`, color **--text-faint**, --fs-sm, user-select none.
  Unified rows render TWO `.line-num` spans (old + new). Context lines add a right border
  `--surface0` on the gutter (`--line-gutter-border`); add/remove leave it transparent.
- **Diff prefix** `.diff-prefix` (`+`/`-`/space): user-select none, `opacity:0.5`.
- **`.diff-add`**: `bg --diff-add-bg` (green 10%), text **--green**, `border-left-color:--green`.
- **`.diff-remove`**: `bg --diff-remove-bg` (red 10%), text **--red**, `border-left-color:--red`.
- **`.diff-context`**: text **--subtext0**, transparent left border.
- When syntax-highlighted (`.diff-line.highlighted`): text → **--text** (tok-* spans color
  individual tokens per §1.3); highlighted context lines drop to `opacity:0.7`.
- **Word-diff** `.word-change`: radius 2; inside add → bg --diff-add-word (green 22%);
  inside remove → bg --diff-remove-word (red 22%).
- **Search match**: `.search-match` bg --search-match-bg (amber 20%) radius 2;
  `.search-match-current` bg --search-match-current-bg (amber 45%) + 1px amber outline.
- **Review bubble** `.review-bubble`: abspos 14px circle in the gutter slack, top 2px,
  `bg --ann-accent` (severity color, default amber), text --base, mono `700 9px/14px`.
  Resolved → hollow green; orphaned → dashed.
- Annotated lines get `box-shadow: inset 3px 0 0 --ann-accent` + an 8%-tint wash; severity
  colors: must-fix=red, suggestion=amber, question=blue, nitpick=surface2, reviewed=green.

### 5.7 Split view
`.split-view`: flex row of two `.split-col` (each flex:1, mono --fs-md, line-height 18,
overflow-x auto, `--diff-gutter-w:6ch` so only one line-num column). `.split-left`
border-right --surface0. Empty/filler rows `.diff-empty`: bg --bg-diff-empty.

### 5.8 Conflict rendering
Conflict regions are framed: `.conflict-line` left/right borders in --conflict-boundary-border,
6px side margins; start/end rows add 2px top/bottom borders + 4px radius + 10px margin.
Region header bar: red→transparent gradient, title `.conflict-region-title` red weight 700
small-caps. Side 1 (diff side): rail color = --conflict-side1-border(red), bg
--conflict-side1-bg; inner +/- preserve green/red. Side 2 (snapshot): rail
--conflict-side2-border, content tinted as green-add. Content lines get a 4px left rail in
the side color. The raw jj marker lines are collapsed to `height:0`.

---

## 6. Misc chrome

### 6.1 Buttons
- **`.btn`** (ghost, default): inline-flex, gap 4, `padding:3px 10px`, transparent bg,
  `1px solid --surface1`, radius 4, color --subtext0, --fs-sm, line-height 1.4. Hover (enabled):
  bg --surface0, color --text, border --surface2. Disabled: opacity 0.35.
- **`.btn-sm`**: `padding:2px 8px`, --fs-xs.
- **`.btn-primary`** (amber filled): bg --amber, border --amber, color **--base**, weight 600;
  hover bg/border `--bg-btn-primary-hover` (≈ #f0ad44).
- **`.btn-danger`** (red outline → fill): color --red, border --red; hover bg --bg-error
  (stays red text/border).
- **`.btn-success`** (green): color --green, border `mix(green 40%,surface1)`, bg `mix(green
  12%,transparent)`, weight 600; hover bg --green, color --crust.
- **`.close-btn`** (`×`): borderless, color --subtext0, --fs-lg, hover --text.

### 6.2 Segmented control
- **`.seg`**: inline-flex, bg --surface0, `1px solid --surface1`, radius 4, overflow hidden.
- **`.seg-btn`**: `padding:3px 10px`, transparent, no border, color --subtext0, --fs-sm;
  hover (inactive) color --text; **`.active`**: bg --bg-active (amber 14%), color --amber, weight 600.
- (RevisionGraph's view-toggle is a lighter analogue: `.view-btn` bg --mantle color --overlay0;
  `.view-btn-active` bg --surface0 color --text.)

### 6.3 Badges & kbd
- **`.nav-hint`** kbd (small inline hint): mono, --fs-2xs, weight 500, color --overlay0,
  no bg, `1px solid --surface1`, `padding:0 3px`, radius 3, margin-left 2 (1 between siblings),
  vertical-align middle.
- **`kbd.key` / `.key-footer kbd`** (bigger chip): inline-block, min-width 14px, `padding:1px 4px`,
  mono, --fs-xs, centered, bg --surface0, `1px solid --surface1`, radius 3, color --text.
- **`.panel-badge`**: pill, bg --surface0, color --subtext0, padding 0 6, radius 8, --fs-xs, weight 600.
- **`.conflict-marker`** `??`: --red weight 600, margin-left 1. **`.sync-marker`** `*`: --amber weight 600.

### 6.4 Modal chrome
- **`.modal-backdrop`**: fixed inset 0, `bg --backdrop` (rgba(0,0,0,0.5)), z 100.
- **`.modal`**: fixed, `top:20%`, centered (left 50% / translateX -50%), `width:520px`,
  `max-height:420px`, bg --base, `1px solid --surface1`, radius 8, `box-shadow:--shadow-heavy`,
  z 101, flex column, overflow hidden.
- **`.modal-header`**: `padding:10px 16px 6px`, --fs-md, weight 700, color --subtext0,
  UPPERCASE, letter-spacing 0.05em, flex space-between baseline.
- **`.modal-input`**: full width, `padding:8px 16px`, bg --mantle, color --text, no border,
  bottom border --surface0, --font-size; placeholder --text-faint.
- **`.key-footer`** (modal hint bar): flex wrap, gap 14, `padding:8px 16px`, top border
  --surface0, --fs-sm, color --subtext0, bg --mantle.

### 6.5 Inline-mode status bar (rebase/squash/split)
When a mode is active the statusbar shows on the left:
- **`.mode-badge`**: bg --amber, color --crust, weight 800, --fs-xs, UPPERCASE,
  letter-spacing 0.08em, `padding:1px 7px`, radius 3.
- A key-group of action keys: `.key` chips (bg --surface0, color --subtext0, 1px --surface1
  border, --fs-xs, weight 600, padding 0 4, radius 3); action keys (`Enter`/`Esc`/`/`)
  use `.action-key` (bg --surface1, color --text); active toggles use `.key-active`
  (bg --amber, color --crust, amber border). `.key-label` --overlay0 --fs-xs; active label
  --text weight 600. `.key-divider`: 1px × 12px --surface1, margin 0 4.
- `.file-count`: --subtext0 --fs-xs; empty-warning (squash, 0 selected) → --red weight 600.

### 6.6 Spinner
`.spinner`: 20×20px, `2px solid --surface0` ring with `border-top-color:--amber`, radius 50%,
0.8s linear rotation. Used in empty/loading states (`.empty-state`: column center, gap 8,
`padding:48px 24px`, color --text-faint, --font-size).

---

## Quick-reference: default (dark) key colors

- App bg `--base #0f0f13`; toolbar/statusbar `--crust #0a0a0e`; panel surfaces `--mantle #0f0f13`.
- Text `--text #e2e2e6`; secondary `--subtext0/--overlay0 #8a8a94`; dim `--text-faint ≈ #71717a`.
- Borders/dividers translucent: `--surface1 (text 7%)`, `--surface2 #4e4e58`.
- Accent `--amber #ffa726` (working-copy ring, selection bar, change-id, primary buttons, active nav/tabs).
- Diff add = `--green #66bb6a` on `green@10%` bg; diff remove = `--red #ef5350` on `red@10%` bg;
  graph lanes are 8 muted hues (`--graph-0..7`, e.g. `#BF8A30…`) drawn at 0.8 (nodes) / 0.45 (lines) opacity.
