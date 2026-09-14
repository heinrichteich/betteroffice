---
"@betteroffice/vsdx": patch
"@betteroffice/vsdx-react": patch
---

Stop the editor erroring on diagrams whose text runs carry no diagnostics: the renderer omits the empty `diagnostics` field, so the type is now optional and the reader guards it.
