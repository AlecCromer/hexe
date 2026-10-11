//! Shared helpers of the row/column surface generators (`torus`, `tube`): Houdini `surftype` connectivity + XYZ rotation.
//!
//! Points are laid out ring by ring: point `(row j, col i)` = `j * cols + i`; columns always wrap; rows wrap for a torus.
//! Connectivity per `surftype` (hython 20.5 probes, torus 3x4 and tube 2x4 / 3x3, `fixtures/sop-torus*`, `sop-tube*`):
//! * `rows` (0): one closed poly per row ring; `cols` (1): one closed poly per column (`rows` points); `rowcol` (2): rows then cols.
//! * `triangles` (3): cell `(a,b,c,d) = (p(i,j), p(i+1,j), p(i+1,j+1), p(i,j+1))` -> `[a,b,c] [a,c,d]`.
//! * `quads` (4): `[a,b,c,d]`. `alttriangles` (5): cells with `(i+j)` even like `triangles`, odd -> `[a,b,d] [b,c,d]`.
//! * `revtriangles` (6): every cell `[a,b,d] [b,c,d]`. Cell order: row-major (row outer, column inner).
use super::meshutil::{MAX_ELEMS, failed};
use hexe_graph::CookError;

pub(super) const SURF_ROWS: u32 = 0;
pub(super) const SURF_COLS: u32 = 1;
pub(super) const SURF_ROWCOL: u32 = 2;
pub(super) const SURF_TRIS: u32 = 3;
pub(super) const SURF_QUADS: u32 = 4;
pub(super) const SURF_ALT: u32 = 5;
pub(super) const SURF_REV: u32 = 6;

/// `surftype` menu shared by torus / tube.
pub(super) const SURF_MENU: [(&str, &str); 7] = [
    ("rows", "Rows"),
    ("cols", "Columns"),
    ("rowcol", "Rows and Columns"),
    ("triangles", "Triangles"),
    ("quads", "Quadrilaterals"),
    ("alttriangles", "Alternating Triangles"),
    ("revtriangles", "Reverse Triangles"),
];

/// Rotation by `deg` about X, then Y, then Z (Houdini `xyz` rotate order).
pub(super) fn rot_xyz(deg: [f64; 3], s: [f64; 3]) -> [f64; 3] {
    let [ax, ay, az] = deg.map(f64::to_radians);
    let (sx, cx) = ax.sin_cos();
    let (sy, cy) = ay.sin_cos();
    let (sz, cz) = az.sin_cos();
    let a = [s[0], s[1] * cx - s[2] * sx, s[1] * sx + s[2] * cx];
    let b = [a[0] * cy + a[2] * sy, a[1], -a[0] * sy + a[2] * cy];
    [b[0] * cz - b[1] * sz, b[0] * sz + b[1] * cz, b[2]]
}

/// Primitives (point ids, always closed polys) of a `rows x cols` point lattice for `surftype`. `wrap_rows`: the last row joins the first.
pub(super) fn surface_prims(
    op: &str,
    rows: u32,
    cols: u32,
    surftype: u32,
    wrap_rows: bool,
) -> Result<Vec<Vec<u32>>, CookError> {
    if u64::from(rows) * u64::from(cols) > MAX_ELEMS {
        return Err(failed(op, "point count exceeds the allocation cap"));
    }
    let p = |i: u32, j: u32| (j % rows) * cols + i % cols;
    let bands = if wrap_rows { rows } else { rows - 1 };
    let mut out = Vec::new();
    let ring_polys = |out: &mut Vec<Vec<u32>>| (0..rows).for_each(|j| out.push((0..cols).map(|i| p(i, j)).collect()));
    let col_polys = |out: &mut Vec<Vec<u32>>| (0..cols).for_each(|i| out.push((0..rows).map(|j| p(i, j)).collect()));
    match surftype {
        SURF_ROWS => ring_polys(&mut out),
        SURF_COLS => col_polys(&mut out),
        SURF_ROWCOL => {
            ring_polys(&mut out);
            col_polys(&mut out);
        }
        _ => {
            for j in 0..bands {
                for i in 0..cols {
                    let (a, b, c, d) = (p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1));
                    let odd_style = match surftype {
                        SURF_TRIS => false,
                        SURF_ALT => (i + j) % 2 == 1,
                        SURF_REV => true,
                        _ => {
                            out.push(vec![a, b, c, d]);
                            continue;
                        }
                    };
                    if odd_style {
                        out.push(vec![a, b, d]);
                        out.push(vec![b, c, d]);
                    } else {
                        out.push(vec![a, b, c]);
                        out.push(vec![a, c, d]);
                    }
                }
            }
        }
    }
    Ok(out)
}
