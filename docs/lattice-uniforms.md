# Named lattice GPU transport

## Transport contract

`uniforms.rs` declares aligned GPU groups with named scalar settings and vector coordinates/colours.
Its local declaration macro emits the Rust structs and test metadata from those same field types and `offset_of!` values.
There is no separately maintained expected-field schema.
The metadata walker starts at the actual Naga uniform binding, then checks every nested field name and offset, scalar kind/width, vector and matrix dimensions, size and alignment, array count and stride, struct span and total binding size.
The alignment comparison uses uniform-address-space requirements (at least 16 bytes for structs and arrays), because Naga retains explicit `@align` in offsets/span while its Layouter reports the member types' natural maximum.
The shader's explicit group alignment matches Rust's `repr(C, align(16))`;
the Pod derive also rejects implicit Rust padding.

Lattice binding 0 starts with `CompositeParams`.
Blit binding 3 reads that 48-byte group,
with bloom at byte 12, the pane background at byte 16 for local shadow composition, and the edge softness at byte 32.
Both bound types are checked against the same Rust declaration,
and the prefix's zero offset, its size and its bloom offset are asserted explicitly.
The roll's and the Spiral's bloom pass their strength in a separate `AddUniforms` buffer at binding 4.

`GlowParams`, row capacity included, is zeroed together when reach or strength disables glow.
Marker world units (`MarkerParams::world_unit`) and the shadow groups sit outside it,
so marker shadows still draw with glow off.
