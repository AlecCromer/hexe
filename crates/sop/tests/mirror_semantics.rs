//! mirror: DOC-VERIFIED against Houdini 20.5 help `sop/mirror.txt` (text only; no Houdini runtime).
//! Doc facts: duplicates + mirrors across plane (origin, dir, dist); point N also mirrored; Reverse Normals reverses
//! the mirrored geometry; Keep Original off => ONLY the selected geometry remains; Consolidate Seam welds points on
//! the plane; Create Output Group puts the mirror prims in a group.
mod common;
use common::*;
use hexe_geo::{AttrClass, AttrData, Attribute, Detail, TypeInfo};
use hexe_graph::ParamValue as PV;
use std::sync::Arc;

fn reflect(n: [f32; 3], d: [f32; 3]) -> [f32; 3] {
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let u = [d[0] / l, d[1] / l, d[2] / l];
    let k = 2.0 * (n[0] * u[0] + n[1] * u[1] + n[2] * u[2]);
    [n[0] - k * u[0], n[1] - k * u[1], n[2] - k * u[2]]
}

#[test]
fn defaults_odd_point_count_doubles_and_negates_across_y0() {
    // default: origin 0, dir +Y, dist 0, keep original on => plane y=0
    let t = tri();
    let o = cook("mirror", &[], Some(&t)).unwrap();
    assert_eq!((o.point_count, o.prims.len()), (6, 2), "3 pts -> 6, 1 prim -> 2");
    assert_eq!(&o.p()[..3], t.p(), "original untouched");
    for i in 0..3 {
        let (a, b) = (t.p()[i], o.p()[3 + i]);
        assert_eq!(b, [a[0], -a[1], a[2]], "mirrored coords negate across the plane");
    }
}

#[test]
fn reverse_normals_toggle_on_a_closed_box() {
    // Reverse on (default): winding fixed so the mirrored normal is the true reflection of the source normal;
    // off: raw reflection flips every polygon normal.
    let c = cube();
    let dir = [1.0, 0.0, 0.0];
    let on = cook("mirror", &[("dir", PV::V3(dir)), ("dist", PV::F(2.0))], Some(&c)).unwrap();
    let off = cook("mirror", &[("dir", PV::V3(dir)), ("dist", PV::F(2.0)), ("reverse", PV::B(false))], Some(&c)).unwrap();
    assert_eq!((on.point_count, on.prims.len()), (16, 12));
    for i in 0..6 {
        let src = newell(&c, i);
        let want = reflect(src, dir);
        let (g, r) = (newell(&on, 6 + i), newell(&off, 6 + i));
        for k in 0..3 {
            assert!(close(g[k], want[k]), "reverse on: prim {i} comp {k}: {g:?} vs {want:?}");
            assert!(close(r[k], -want[k]), "reverse off: prim {i} comp {k}");
        }
    }
    // plane at x=2: cube [-.5,.5] -> [3.5,4.5]
    let (lo, hi) = on.bbox();
    assert_eq!((lo[0], hi[0]), (-0.5, 4.5));
}

#[test]
fn point_normals_are_mirrored_too() {
    let mut d = (*tri()).clone();
    d.add_attr(Attribute {
        name: "N".into(),
        class: AttrClass::Point,
        data: AttrData::V3(vec![[0.0, 1.0, 0.0], [0.6, 0.8, 0.0], [0.0, 0.0, 1.0]]),
        type_info: TypeInfo::Normal,
    });
    let o = cook("mirror", &[], Some(&Arc::new(d))).unwrap();
    let n = v3(&o, AttrClass::Point, "N");
    assert_eq!(n[3], [0.0, -1.0, 0.0]);
    assert_eq!(n[4], [0.6, -0.8, 0.0]);
    assert_eq!(n[5], [0.0, 0.0, 1.0]);
}

#[test]
fn keep_original_off_leaves_only_selected_geometry() {
    // doc: "Keep Original: ... When off, only the selected geometry will remain."
    let g = grid3();
    let o = cook("mirror", &[("group", s("0")), ("keepOriginal", PV::B(false))], Some(&g)).unwrap();
    assert_eq!(o.prims.len(), 1, "only the selected prim remains");
    let pts = prim_pts(&o, 0);
    for p in pts {
        assert!(o.p()[p as usize][1] <= 0.0, "mirrored below y=0");
    }
    // no group => everything is selected: same prim count as input, all reflected in place
    let all = cook("mirror", &[("keepOriginal", PV::B(false))], Some(&g)).unwrap();
    assert_eq!((all.point_count, all.prims.len()), (9, 4));
}

#[test]
fn consolidate_seam_welds_points_on_the_plane() {
    // grid3 y in 0..2, plane y=0: the 3 y=0 points coincide with their mirror copies
    let g = grid3();
    let plain = cook("mirror", &[], Some(&g)).unwrap();
    assert_eq!(plain.point_count, 18);
    let c = cook("mirror", &[("consolidate", PV::B(true))], Some(&g)).unwrap();
    assert_eq!(c.point_count, 15, "3 seam points welded");
    assert_eq!(c.prims.len(), 8);
}

#[test]
fn output_group_holds_exactly_the_mirror_prims() {
    let g = grid3();
    let o = cook("mirror", &[("createoutputgroup", PV::B(true)), ("outputgroup", s("mir"))], Some(&g)).unwrap();
    let grp = o.group(AttrClass::Prim, "mir").expect("output group");
    let sel: Vec<usize> = grp.mask.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect();
    assert_eq!(sel, [4, 5, 6, 7]);
}

#[test]
fn empty_input_and_zero_dir() {
    let e = Arc::new(Detail::default());
    let o = cook("mirror", &[], Some(&e)).unwrap();
    assert_eq!((o.point_count, o.prims.len()), (0, 0));
    assert!(cook("mirror", &[("dir", PV::V3([0.0; 3]))], Some(&grid3())).is_err(), "zero normal is an error");
}
