---
"@betteroffice/rust-crates": patch
---

Resolve group sub-shapes that carry `MasterShape` without their own `Master`. The resolver passed
the PageSheet where a shape-lookup sheet was needed, and a PageSheet holds no shapes, so every such
sub-shape inherited nothing and rendered as a placeholder.
