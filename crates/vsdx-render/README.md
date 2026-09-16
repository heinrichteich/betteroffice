# betteroffice-vsdx-render

Dynamic connectors resolve their ShapeSheet endpoint values and page glue records at layout time.
The routing policy is deterministic: ordinary connectors are straight, while nonzero `RoutStyle`
uses a horizontal-first orthogonal bend. It does not emulate Visio obstacle avoidance
or manually edited route geometry; an unresolved route becomes a placeholder instead of a guessed line.

Connector crossings render Visio line jumps at layout time: the more-horizontal leg bridges the
more-vertical one under `LineJumpCode` 1 (mirrored for 2, z-order for 4 and 5), sized by
`LineJumpFactorX/Y` times `LineToLineX/Y`, with per-connector `ConLineJumpCode` overrides.
Arc and gap styles are supported; other jump styles and last-routed (3) crossings stay unbridged.

VSDX resolved-scene to display-list compiler and hit tester.

Geometry sections keep their individual `IX` identities. Enabled or unevaluable
section-level `NoFill`, `NoLine`, and `NoShow` controls produce an explicit
unsupported placeholder; their paint semantics are not yet implemented. Disabled
controls remain renderable. The original section XML remains preserved on save.
