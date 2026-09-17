---
"@betteroffice/vsdx-react": patch
"@betteroffice/vsdx-i18n": patch
---

Add a Drawing Explorer panel: a tree of the document's pages, shapes and group children with each shape's ShapeSheet sections, rows, formulas and values. It mirrors the engine snapshot instead of keeping its own model and shares the canvas selection, and it refuses to select a shape hidden by its layer.
