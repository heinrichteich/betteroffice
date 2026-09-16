# betteroffice-vsdx-render

Dynamic connectors resolve their ShapeSheet endpoint values and page glue records at layout time.
The routing policy is deterministic: a single-subpath filed `Geometry` supplies the waypoints,
anchored to the resolved endpoints, otherwise the shape's `ShapeRouteStyle` — or the page's
`RouteStyle` — selects a direct run or one horizontal-first or vertical-first orthogonal bend.
It does not emulate Visio obstacle avoidance or line jumps; an unresolved route becomes a
placeholder instead of a guessed line.

VSDX resolved-scene to display-list compiler and hit tester.

Geometry sections keep their individual `IX` identities. Enabled or unevaluable
section-level `NoFill`, `NoLine`, and `NoShow` controls produce an explicit
unsupported placeholder; their paint semantics are not yet implemented. Disabled
controls remain renderable. The original section XML remains preserved on save.
