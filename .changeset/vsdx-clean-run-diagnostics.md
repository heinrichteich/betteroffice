---
"@betteroffice/vsdx": patch
"@betteroffice/vsdx-react": patch
---

Stop the editor erroring on diagrams whose text runs carry no diagnostics. The renderer omits the
`diagnostics` field entirely for a clean run, so the TypeScript type now declares it optional and
the reader guards it, matching how `Stroke.dashed` already works.
