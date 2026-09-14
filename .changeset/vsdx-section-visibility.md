---
"@betteroffice/rust-crates": patch
"@betteroffice/vsdx": patch
---

Honour Geometry section `NoFill`, `NoLine` and `NoShow` controls when rendering shapes and connectors.
Render visible geometry with only the paint channels it uses, while retaining diagnostics for unsupported controls.
