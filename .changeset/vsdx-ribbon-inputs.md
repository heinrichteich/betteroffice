---
"@betteroffice/vsdx": patch
"@betteroffice/vsdx-react": patch
"@betteroffice/vsdx-i18n": patch
"@betteroffice/rust-crates": patch
---

Make the ribbon's line and colour controls behave: the line-weight field validates its text and commits once on Enter or blur, line pattern is a bounded picker, and the fill and line swatches read the rendered colour so a palette or theme fill no longer shows as black. Rewriting a cell with the formula it already holds no longer records an undo entry or sends a collaboration update, a non-positive `LineWeight` falls back to the default stroke width instead of an unpaintable one, and a guarded `Angle` hides the rotation grip and refuses the gesture with a receipt instead of committing on release.
