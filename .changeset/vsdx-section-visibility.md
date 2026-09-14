---
"@betteroffice/rust-crates": patch
---

Honour a Geometry section's `NoFill`, `NoLine` and `NoShow` controls instead of replacing the whole
shape with a placeholder. Each Geometry section now emits its own primitive carrying only the paint
its controls allow, so a hidden section no longer hides its siblings.
