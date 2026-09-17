---
"@betteroffice/vsdx": minor
"@betteroffice/vsdx-react": patch
"@betteroffice/rust-crates": patch
---

Render Visio linear fill gradients. The renderer resolves the `FillGradient` section through masters and styles, evaluates `GradientStopColor` with the theme so `THEMEVAL` stops pick up the document theme, and emits a gradient paint carrying `FillGradientAngle`. Non-linear `FillGradientDir` values, an unresolvable stop and gradients with fewer than two stops fall back to the solid fill with a fidelity diagnostic; a stop with `GradientStopColorTrans` is painted opaque and reported. The display-list contract version moves from 4 to 5 and `Paint` gains an optional `angleDeg`.
