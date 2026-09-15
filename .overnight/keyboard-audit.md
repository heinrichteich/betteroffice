# VSDX keyboard + focus audit (U2)

Branch: `chore/vsdx-keyboard-audit` (= `origin/fix/vsdx-document-keys`, HEAD `f8c8c3f4`).
Visio reference: round-3 audit (`ui-audit-round3.md`) + Visio's own `Hilfe > Tastenkombinationen`
(9 categories: Alle Verknüpfungen, Häufig verwendete, Arbeiten mit Formen, Einfügemarke im Text,
Inhalt auswählen, Menübandnavigation, Mindmap, Seitennavigation, Textformatierung).

Bindings below were found in code first (`packages/vsdx-react/src/interactions.ts`
`canvasKeyboardIntent`, `VsdxEditor.tsx` document fallback, `CommandMenu.tsx`, `Ribbon.tsx`,
`ShapesPanel.tsx`, `StatusBar.tsx`, `ShapeContextMenu.tsx`, `commands.ts`) — nothing guessed.

## Headline findings (read these, skip the rest if in a hurry)

1. **Multi-shape Delete needs one undo per shape.** Ctrl+A, Delete with 3 shapes selected:
   a single Ctrl+Z restores exactly 1 shape; full restore needs 3 undos. Same for multi-shape
   arrow-nudge: one press with N shapes = N undo steps in the browser. Single-shape edits are
   exactly 1 step. Root cause: one Yjs txn per shape, and the wasm undo clock ticks past the
   500 ms coalescing window on every call, so synchronous multi-shape gestures never coalesce
   (native builds would merge them). Visio restores the whole gesture in one undo.
2. **Focus falls out of the editor after a ribbon delete.** Deleting from the ribbon disables the
   Delete button under the focused element, so focus drops to `<body>`; a subsequent Ctrl+Z
   goes nowhere (target outside the editor root) until the user Tabs back in. Verified in
   headless Chrome: `activeElement` = BODY, Ctrl+Z a no-op, undo stack untouched.
3. **Tab never cycles shapes** (Visio: Tab / Shift+Tab = next / previous object). Tab only walks
   DOM focus. Canvas is reachable (`tabIndex=0`) and escapable, so no trap — but the binding
   is missing.
4. **No keyboard path to the context menu.** Trusted Shift+F10 and the Menu key open nothing.
   Keyboard-only users can still reach every context-menu command through the ribbon, so this
   is parity, not a blocker.
5. **Leaks to the browser:** Ctrl+D opens the bookmark dialog (Visio: duplicate), Ctrl+S opens
   save-page (Visio: save; we have download but no key). Ctrl+P (print dialog) and Ctrl+F
   (find) also pass through — acceptable until print/search exist.
6. **Shift+arrows follow the platform convention, Visio inverts it.** Ours: plain arrows = 1 px,
   Shift = x10. Visio: plain = coarse step, Shift = fine 1 px. Flagging as a deliberate-looking
   difference, not a bug per se.
7. **Nothing destructive lacks an undo step.** Delete, nudge, drag, resize, rotate, flip,
   recolour, arrange, insert, page reorder all go through undo-tracked txns (verified Delete
   and nudge end to end; rest by code). Granularity caveats above are the only wart.
8. **No code fixed.** Audit only; one report file.

## Table: Binding | Context | Ours | Visio | Verdict

Wrong or missing first, then correct behaviour, then focus mechanics.

| Binding | Context | Ours | Visio | Verdict |
|---|---|---|---|---|
| Delete, N shapes selected | canvas / ribbon focus | deletes all N; **one undo restores 1 shape** (browser-verified: 3 deletes needed 3 undos) | one undo restores all | wrong (granularity) |
| Arrow press, N shapes selected | canvas / ribbon focus | N undo steps per press (browser) | one undo step | wrong (granularity) |
| Ctrl+Z after ribbon delete | ribbon focus, button just disabled | focus is on `<body>`; Ctrl+Z does nothing | focus stays usable, undo works | wrong (focus loss) |
| Tab / Shift+Tab | canvas focus | moves DOM focus to status bar; selection untouched | cycles next / previous shape | missing |
| Shift+F10, Menu key | canvas / shape focus | nothing (trusted keys) | context menu | missing |
| Enter / Space | canvas focus | nothing (trusted keys, no scroll, selection kept) | text edit | missing (no text editing at all) |
| Ctrl+D | editor focus | browser bookmark dialog | duplicate shape | missing (leak) |
| Ctrl+S | editor focus | browser save-page dialog | save | missing (leak; download exists, no key) |
| Ctrl+C / X / V | shape selected | silently nothing | copy / cut / paste | missing (gap, not leak) |
| Ctrl+G, Ctrl+Shift+U | anywhere | nothing | group / ungroup | missing (no grouping) |
| Ctrl+R / Ctrl+L | anywhere | browser reload / address bar | rotate 90 CW / CCW | missing (Ctrl+L not interceptable in Chrome) |
| Ctrl+Shift+F / B | anywhere | nothing | bring forward / send backward | missing (menu only) |
| Ctrl+Shift+C / V | anywhere | nothing | format-painter copy / paste | missing (no painter) |
| Ctrl+arrows | canvas focus | nothing | AutoConnect suggestions + Quick Shapes | missing |
| Alt+3 | anywhere | nothing | connector tool | missing (no tool modes) |
| Tab | page-list menu open | menu stays open, focus walks away (Escape still closes it) | n/a | wrong (minor) |
| Escape | page-list menu open, canvas focused | closes menu AND clears selection (two handlers, neither defers) | staged: close first, clear second | partial (minor overlap) |
| Shift+arrows | canvas focus | x10 coarse nudge (browser-verified: +0.104" vs +0.0104") | fine 1 px nudge (inverted convention) | differs (platform convention, flag) |
| Ctrl+A | canvas / ribbon focus | selects all shapes (browser-verified, label "3 shapes selected") | select all | present |
| Ctrl+A | text input / outside editor | passes to input / browser (browser-verified: search text selected, doc untouched) | n/a | present |
| Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z | canvas / ribbon focus | undo / redo (browser-verified both directions) | undo / redo | present |
| Ctrl+Z | text input focus | passes to input; document history untouched (unit-tested) | n/a | present |
| Delete / Backspace | canvas / ribbon focus | deletes selection; refused when LockDelete/GUARD (code + unit tests) | delete | present |
| Delete / Backspace | text input focus | passes to input; shape survives (browser + unit-tested) | n/a | present |
| Delete / Backspace | nothing selected | preventDefault + no-op; no crash, undo stack untouched (browser-verified) | n/a | present |
| Escape | canvas focus | cancels drag (no commit), closes menu, clears selection (unit-tested) | leave edit, then clear | present (no text edit, so single stage is correct) |
| Escape | ribbon / tab focus | clears selection, focus stays on enabled control (browser-verified) | clear selection | present |
| Escape | open menu | closes menu, refocuses trigger/canvas (browser-verified both menus) | close menu | present |
| Arrows | canvas / ribbon focus | 1 px nudge at zoom 1, zoom-scaled (browser-verified PinY +1/96) | move shape | present (step convention differs, see above) |
| F2 | anywhere | nothing | not a Visio binding (Enter/Space is) | present (correctly absent) |
| Ctrl+F / Ctrl+P | editor focus | browser find / print dialog | n/a (web app leaves these to the browser too) | pass-through, acceptable |
| Ctrl+C/X/V/G, Ctrl+Shift+* | text input focus | pass to input (code: editable-target guard) | n/a | present |
| ArrowUp/Down/Home/End | context / ribbon-split menu | move between enabled items, wrap (trusted ArrowDown verified) | menu navigation | present |
| ArrowRight/Enter/Space, ArrowLeft | menu with submenu | open / close submenu (code; Enter path not measurable, see method) | submenu navigation | present (modulo method) |
| ArrowLeft/Right/Home/End | ribbon tabs | move + select tab, wrap (code, roving tabindex) | ribbon navigation | present |
| Arrows/Home/End | shapes grid | move tile focus, wrap (code, roving tabindex) | stencil navigation | present |
| Enter / Space | shape tile focus | Space inserts shape (browser-verified, undoable); Enter not measurable by this method | Enter inserts focused stencil shape | partial (method limit, likely present) |
| ArrowLeft/Right/Home/End | page tabs | move focus AND select page (code) | page navigation | present |
| Arrows | zoom slider focus | native slider step; no shape nudge (browser-verified: slider 500->501, PinY unchanged) | n/a | present |
| Click | shape tile | inserts shape at canvas centre, undoable (browser-verified, no errors) | insert shape | present |

Focus reachability (Tab walk, trusted, 25 stops, no traps found):

| Check | Result |
|---|---|
| Keyboard reaches canvas | yes, `tabIndex=0` (unit + browser) |
| Keyboard escapes canvas | yes, Tab moves to status bar; selection untouched |
| Keyboard reaches every control | yes: tiles, canvas, page list/nav/tabs, zoom controls, ribbon tabs + panel buttons, site chrome; full walk recorded, no `TAB-PREVENTED` |
| Focus trap | none found; page-list menu left open by Tab is the only focus-leak (row above) |
| Menu focus | context/split menus auto-focus first enabled item; Escape refocuses trigger/canvas |
| Disabled-control focus | Delete button unfocusable while disabled (expected); the bug is no fallback focus target (row 3) |

## Method (so the next audit doesn't re-learn this)

- Own server: `next dev -p 3781` in this worktree (3781 free; 3762/3763 served other branches).
  Headless Chromium 151 (playwright cache) over raw CDP WebSocket, `Input.dispatchKeyEvent`
  (trusted) for behaviour, synthetic `KeyboardEvent` only for the preventDefault map — valid
  because our path checks neither `isTrusted` nor anything synthetic events lack.
- **Stale-dist trap, first run invalidated:** `next dev` does NOT compile vsdx packages from
  source. `main: ./dist/index.js` is resolved and merely transpiled, so the browser ran a
  15:57 dist predating the document-keyboard commit (Ctrl+A dead, no banner). Rebuilt
  `vsdx-react` + `vsdx-i18n` dists from current source and reran everything; `dist/` is
  gitignored, tree untouched. The brief's "compiles them from source" claim is wrong for
  these packages.
- **Enter activation not measurable:** CDP `rawKeyDown` Enter does not trigger native button
  activation in this Chrome (control failed on the zoom-in button too, while Space works).
  Tile-Enter is therefore "method could not measure", not "does nothing".
- **Forbidden keys untested by design** (Ctrl+W/T/N, Alt+F4, F11); our handler maps none of
  them, so they reach the browser by construction.
- **Hover untested** per the round-3 retraction rule (synthetic hover is not evidence).
- **Banner:** `u.diagnostics is not iterable` IS on this branch (`6fd00306`, PR #432, is not
  an ancestor of HEAD) — already fixed upstream, not reported as live.
- Suite state: `bun test packages/vsdx-react/src/interactions.test.ts
  packages/vsdx-react/src/components packages/vsdx-i18n` → 94 pass; lifecycle file 33/34 —
  the single failure (`restoring earlier bytes for the same font face registers them again`,
  FontFace timeout under happy-dom) is pre-existing on this branch and unrelated to keyboard.
  No source file was changed by this audit, so everything failing/succeeding is as-found.
- A killed, stillborn `vsdx` package build briefly started a wasm rebuild; it was stopped
  before writing anything (`git status` clean, no cargo legs run — no Rust touched).
