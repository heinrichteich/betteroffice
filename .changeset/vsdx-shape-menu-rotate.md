---
"@betteroffice/vsdx-react": patch
---

Add a rotate and flip submenu to the shape context menu, dispatching the ribbon's existing rotate-left, rotate-right, flip-horizontal and flip-vertical commands. A guarded Angle, FlipX or FlipY disables the matching entry, and a shape whose rotation and both flips are guarded disables the submenu trigger.
