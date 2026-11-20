# attrs.layout

`attrs.layout.packed` and `attrs.layout.packed_layout(.enabled = true)` produce `{ packed: bool }` attribute Objects consumed by struct layout checking (`size = sum` of field sizes, `align = 1`, taking aligned field pointers forbidden).
