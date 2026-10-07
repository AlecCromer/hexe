//! Data model. DESIGN.md §crates/geo contract + the deviations listed in crates/geo/README.md.

pub type PointId = u32;
pub type VertexId = u32;
pub type PrimId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrClass {
    Point,
    Vertex,
    Prim,
    Detail,
}

/// Houdini attribute type info (`options.type.value`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TypeInfo {
    Point,
    Vector,
    Normal,
    Color,
    Quaternion,
    #[default]
    None,
}

/// DEVIATION: `FN`/`IN` added (tuple size outside 1..=4 for floats, >1 for ints) so no attribute is ever dropped.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrData {
    F32(Vec<f32>),
    I32(Vec<i32>),
    V2(Vec<[f32; 2]>),
    V3(Vec<[f32; 3]>),
    V4(Vec<[f32; 4]>),
    Str { idx: Vec<i32>, table: Vec<String> },
    FN { size: u32, data: Vec<f32> },
    IN { size: u32, data: Vec<i32> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub class: AttrClass,
    pub data: AttrData,
    pub type_info: TypeInfo,
}

/// DEVIATION: `Sphere`/`Tube` carry their 3x3 transform (row-major) and `Unknown` carries the verbatim source JSON, so
/// read→write loses nothing. `Volume`/`PackedRef` stay unit (read as `Unknown`).
#[derive(Clone, Debug, PartialEq)]
pub enum PrimKind {
    Poly { closed: bool },
    Polyline,
    Sphere { xform: [f32; 9] },
    Tube { xform: [f32; 9], taper: f32, closed: bool },
    Volume,
    PackedRef,
    /// Passthrough for any primitive type we do not model. `json` = serialised source entry; `ty` = its type key.
    Unknown { ty: String, json: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prim {
    pub kind: PrimKind,
    /// Indexes into `Detail.vertex_point` (vertex ids, NOT point ids). Empty for `Unknown` whose vertices could not be modelled.
    pub verts: std::ops::Range<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    /// `Point` | `Vertex` | `Prim` (`Detail` is invalid for groups).
    pub class: AttrClass,
    pub mask: Vec<bool>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub point_count: u32,
    pub vertex_point: Vec<PointId>,
    pub prims: Vec<Prim>,
    pub attrs: Vec<Attribute>,
    pub groups: Vec<Group>,
}
