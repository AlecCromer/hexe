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

/// Spline basis of one parametric direction (NURBS/Bezier curve or one direction of a surface).
/// * NURBS: `knots` = FULL knot vector (`cvs + order` open, `cvs + 2*order - 1` closed/periodic), non-decreasing; empty or
///   wrong length = default (open+`end`: clamped uniform; otherwise uniform; closed: periodic uniform). `end` = end interpolation.
/// * Bezier (piecewise, degree `order-1`, shared end CVs): `knots` = segment break parameters (`segments + 1` values,
///   increasing; empty = uniform); `end` ignored.
///
/// Rational weights are the point attribute `Pw` (absent = 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Basis {
    pub order: u32,
    pub knots: Vec<f32>,
    pub end: bool,
}

impl Basis {
    pub fn new(order: u32) -> Basis {
        Basis { order, knots: Vec::new(), end: true }
    }
}

/// DEVIATION: `Sphere`/`Tube` carry their 3x3 transform (row-major) and `Unknown` carries the verbatim source JSON, so
/// read→write loses nothing. `Volume`/`PackedRef` stay unit (read as `Unknown`).
#[derive(Clone, Debug, PartialEq)]
pub enum PrimKind {
    Poly {
        closed: bool,
    },
    Polyline,
    Sphere {
        xform: [f32; 9],
    },
    Tube {
        xform: [f32; 9],
        taper: f32,
        closed: bool,
    },
    Volume,
    PackedRef,
    /// Piecewise Bezier curve over the prim's vertices (`closed` = last segment ends on the first CV).
    BezierCurve {
        basis: Basis,
        closed: bool,
    },
    /// NURBS curve (rational iff point attr `Pw` present).
    NurbsCurve {
        basis: Basis,
        closed: bool,
    },
    /// Piecewise Bezier surface: `cols x rows` CVs, vertex `r*cols + c` (u along `c`, v along `r`); `uwrap`/`vwrap` = closed.
    BezierSurf {
        cols: u32,
        rows: u32,
        ubasis: Basis,
        vbasis: Basis,
        uwrap: bool,
        vwrap: bool,
    },
    /// NURBS surface, same CV layout as `BezierSurf`.
    NurbsSurf {
        cols: u32,
        rows: u32,
        ubasis: Basis,
        vbasis: Basis,
        uwrap: bool,
        vwrap: bool,
    },
    /// Passthrough for any primitive type we do not model. `json` = serialised source entry; `ty` = its type key.
    Unknown {
        ty: String,
        json: String,
    },
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
    /// Attributes present in the source but with no `AttrData` form (`dict`, `stringarray`, ...): `(class, name, source type)`.
    /// Never silent: readers push here instead of dropping. Not written back by `write_geo_json`.
    pub skipped_attribs: Vec<(AttrClass, String, String)>,
}
