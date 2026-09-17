---
"@betteroffice/vsdx-react": patch
"@betteroffice/vsdx-i18n": patch
---

Show a fill and line colour mini-toolbar above the shape context menu, flipping below the menu when it would leave the viewport and sharing the menu's Escape, outside-press and command-selection lifetime. A GUARD on FillForegnd or LineColor now disables that colour command everywhere it is offered, including the ribbon, instead of letting the pick reach the mutation policy and be refused. Open menus and submenus re-clamp when the window resizes.
