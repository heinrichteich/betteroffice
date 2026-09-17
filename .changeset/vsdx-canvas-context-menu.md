---
"@betteroffice/vsdx-react": patch
"@betteroffice/vsdx-i18n": patch
---

Open a context menu on an empty-canvas right-click with undo, redo and add shape, drawn only from commands the ribbon registry already carries. The menu clears the selection, stays inside the viewport, closes on Escape, on an outside press, on a page switch and when the document is replaced. A hover-opened submenu no longer closes when its trigger is clicked, and command enablement now matches a GUARD call rather than any formula containing the letters GUARD.
