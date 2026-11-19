//! Test-only reflection of the transport declarations in the parent module.

use super::{Float2, Float4, Int2, Matrix4, Uint4};

pub(crate) struct Layout {
    size: usize,
    alignment: usize,
    kind: Kind,
}

pub(crate) enum Kind {
    Scalar(naga::Scalar),
    Vector(naga::Scalar, naga::VectorSize),
    Matrix(naga::Scalar, naga::VectorSize, naga::VectorSize),
    Array { element: Box<Layout>, count: usize, stride: usize },
    Struct(Vec<Field>),
}

pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) offset: usize,
    pub(crate) layout: Layout,
}

/// Which address space a binding is read from, which decides the alignment
/// rule [`Layout::check`] holds a declaration to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    Uniform,
    Storage,
}

pub(crate) trait GpuLayout {
    fn layout() -> Layout;
}

impl Layout {
    pub(crate) fn of<T>(kind: Kind) -> Self {
        Self { size: size_of::<T>(), alignment: align_of::<T>(), kind }
    }

    fn check(
        &self,
        module: &naga::Module,
        resolved: &naga::proc::Layouter,
        ty: naga::Handle<naga::Type>,
        path: &str,
        prefix: bool,
        space: Space,
    ) {
        use naga::TypeInner as T;
        let layout = resolved[ty];
        if prefix {
            assert!(self.size >= layout.size as usize, "{path}: prefix fits the uploaded buffer");
        } else {
            assert_eq!(self.size, layout.size as usize, "{path}: size");
        }
        // Naga retains @align's offsets/span but its Layouter reports the
        // member types' natural maximum. Uniform structs/arrays instead
        // require at least 16-byte alignment (valid/type.rs applies this too).
        // Storage takes the natural alignment, which a plain `repr(C)` row
        // need not share: a row of `[f32; 4]` aligns to 4 where its `vec4`
        // aligns to 16. What the bytes depend on there is every offset, size
        // and stride, which are all checked, so the alignment is not.
        if space == Space::Uniform {
            let required_alignment = match module.types[ty].inner {
                T::Struct { .. } | T::Array { .. } => {
                    layout.alignment.max(naga::proc::Alignment::MIN_UNIFORM)
                }
                _ => layout.alignment,
            };
            assert_eq!(
                naga::proc::Alignment::new(self.alignment as u32).unwrap(),
                required_alignment,
                "{path}: required uniform alignment"
            );
        }
        match (&self.kind, &module.types[ty].inner) {
            (Kind::Scalar(expected), T::Scalar(actual)) => {
                assert_eq!(expected, actual, "{path}: scalar kind/width");
            }
            (Kind::Vector(scalar, size), T::Vector { scalar: actual, size: actual_size }) => {
                assert_eq!(scalar, actual, "{path}: vector scalar kind/width");
                assert_eq!(size, actual_size, "{path}: vector lanes");
            }
            (
                Kind::Matrix(scalar, columns, rows),
                T::Matrix { scalar: actual, columns: actual_columns, rows: actual_rows },
            ) => {
                assert_eq!(scalar, actual, "{path}: matrix scalar kind/width");
                assert_eq!(columns, actual_columns, "{path}: matrix columns");
                assert_eq!(rows, actual_rows, "{path}: matrix rows");
            }
            (Kind::Array { element, count, stride }, T::Array { base, size, stride: actual }) => {
                assert_eq!(
                    *size,
                    naga::ArraySize::Constant((*count as u32).try_into().unwrap()),
                    "{path}: array count"
                );
                assert_eq!(*stride, *actual as usize, "{path}: array stride");
                element.check(module, resolved, *base, &format!("{path}[]"), false, space);
            }
            (Kind::Struct(fields), T::Struct { members, span }) => {
                if prefix {
                    assert!(self.size >= *span as usize, "{path}: struct prefix span");
                    assert!(fields.len() >= members.len(), "{path}: struct prefix fields");
                } else {
                    assert_eq!(self.size, *span as usize, "{path}: struct span");
                    assert_eq!(fields.len(), members.len(), "{path}: field count");
                }
                for (field, member) in fields.iter().zip(members) {
                    let path = format!("{path}.{}", field.name);
                    assert_eq!(Some(field.name), member.name.as_deref(), "{path}: field name");
                    assert_eq!(field.offset, member.offset as usize, "{path}: offset");
                    field.layout.check(module, resolved, member.ty, &path, false, space);
                }
            }
            _ => panic!("{path}: Rust transport and WGSL type shapes differ"),
        }
    }
}

impl GpuLayout for f32 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Scalar(naga::Scalar::F32))
    }
}
impl GpuLayout for u32 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Scalar(naga::Scalar::U32))
    }
}
impl GpuLayout for i32 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Scalar(naga::Scalar::I32))
    }
}
impl GpuLayout for Int2 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Vector(naga::Scalar::I32, naga::VectorSize::Bi))
    }
}
impl GpuLayout for Float2 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Vector(naga::Scalar::F32, naga::VectorSize::Bi))
    }
}
impl GpuLayout for Float4 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Vector(naga::Scalar::F32, naga::VectorSize::Quad))
    }
}
impl GpuLayout for Uint4 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Vector(naga::Scalar::U32, naga::VectorSize::Quad))
    }
}
impl GpuLayout for Matrix4 {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Matrix(
            naga::Scalar::F32,
            naga::VectorSize::Quad,
            naga::VectorSize::Quad,
        ))
    }
}
impl<T: GpuLayout, const N: usize> GpuLayout for [T; N] {
    fn layout() -> Layout {
        Layout::of::<Self>(Kind::Array {
            element: Box::new(T::layout()),
            count: N,
            stride: size_of::<T>(),
        })
    }
}

/// Start at the bound type, so an unused correct-looking declaration cannot
/// satisfy the contract while the pipeline actually consumes another struct.
pub(crate) fn check_binding<T: GpuLayout>(source: &str, group: u32, binding: u32) {
    check_bound::<T>(source, group, binding, false);
}

/// A shader may read only the head of an existing upload. Check that head
/// against the actual declaration rather than maintaining a second Rust mirror.
pub(crate) fn check_binding_prefix<T: GpuLayout>(source: &str, group: u32, binding: u32) {
    check_bound::<T>(source, group, binding, true);
}

fn check_bound<T: GpuLayout>(source: &str, group: u32, binding: u32, prefix: bool) {
    let (module, resolved) = parse(source);
    let variable = bound(&module, group, binding, |space| space == naga::AddressSpace::Uniform);
    let path = format!("{group}:{binding}");
    T::layout().check(&module, &resolved, variable.ty, &path, prefix, Space::Uniform);
}

/// A storage binding read as `array<T>`: a runtime-sized array whose stride is
/// one Rust row, of elements laid out as `T` is.
///
/// Its rows are packed four bytes apart at the finest, so a field moved or
/// added on one side shifts every field after it into the next row's bytes
/// without any validation error.
pub(crate) fn check_storage_array<T: GpuLayout>(source: &str, group: u32, binding: u32) {
    let (module, resolved) = parse(source);
    let variable =
        bound(&module, group, binding, |space| matches!(space, naga::AddressSpace::Storage { .. }));
    let path = format!("{group}:{binding}");
    let naga::TypeInner::Array { base, size, stride } = module.types[variable.ty].inner else {
        panic!("{path}: a storage array");
    };
    assert_eq!(size, naga::ArraySize::Dynamic, "{path}: runtime-sized");
    assert_eq!(stride as usize, size_of::<T>(), "{path}: array stride");
    T::layout().check(&module, &resolved, base, &format!("{path}[]"), false, Space::Storage);
}

fn parse(source: &str) -> (naga::Module, naga::proc::Layouter) {
    let module = naga::front::wgsl::parse_str(source).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let mut resolved = naga::proc::Layouter::default();
    resolved.update(module.to_ctx()).unwrap();
    (module, resolved)
}

/// The one variable at `group:binding` in the address space `space` accepts.
/// Filtered on the space too, since a shader may bind a texture at the same
/// slot as a buffer for entry points of its own.
fn bound(
    module: &naga::Module,
    group: u32,
    binding: u32,
    space: impl Fn(naga::AddressSpace) -> bool,
) -> &naga::GlobalVariable {
    let variables: Vec<_> = module
        .global_variables
        .iter()
        .filter(|(_, var)| var.binding == Some(naga::ResourceBinding { group, binding }))
        .filter(|(_, var)| space(var.space))
        .collect();
    assert_eq!(variables.len(), 1, "one variable at {group}:{binding}");
    variables[0].1
}
