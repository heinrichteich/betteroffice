# VSDX office export

Headless export of Visio diagrams to PowerPoint (`.pptx`) and Word (`.docx`).

One slide per page carries the diagram as native DrawingML shapes translated
from the `vsdx-render` display list, so output stays editable. The Word
document carries the same shapes plus a table of shape data per page.

Shape data is the resolved `Property` section; rows without a cached value
or formula are omitted. Gradients export as vertical linear blends, text
re-flows in the host, and placeholders render as dashed boxes labelled with
their reason.
