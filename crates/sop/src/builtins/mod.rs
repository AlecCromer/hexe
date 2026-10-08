//! THE merge point for SOP lanes. A lane makes `builtins/<name>_sop.rs` and touches exactly TWO lines here: its `mod` slot
//! and its `register` slot. Slots are pre-seeded, alphabetical, separated by `//` lines so parallel lanes never
//! produce adjacent hunks: uncomment YOUR two lines, nothing else. New type not listed → insert a line in its
//! sorted position between separators.
use crate::Registry;

mod null_sop;
//
mod add_sop;
//
mod attribs;
//
mod attribcopy_sop;
//
mod attribcreate_sop;
//
mod attribdelete_sop;
//
mod attribnoise_sop;
//
mod attribpromote_sop;
//
mod attribtransfer_sop;
//
mod bend_sop;
//
mod blast_sop;
//
mod block_sop;
mod boolean_sop;
//
mod box_sop;
//
mod circle_sop;
//
mod clip_sop;
//
mod color_sop;
//
mod connectivity_sop;
//
mod copytopoints_sop;
//
mod curve_sop;
//
mod delete_sop;
//
mod divide_sop;
//
mod fuse_sop;
//
mod facet_sop;
//
mod file_sop;
//
mod grid_sop;
//
mod group_sop;
//
mod groupdelete_sop;
//
mod grouppromote_sop;
//
mod knife_sop;
//
mod line_sop;
//
mod matchsize_sop;
//
mod measure_sop;
//
mod merge_sop;
//
mod mirror_sop;
//
mod mountain_sop;
//
mod normal_sop;
//
mod meshutil;
//
mod selutil;
//
mod output_sop;
//
mod peak_sop;
//
mod polybevel_sop;
//
mod polyextrude_sop;
//
mod ray_sop;
//
mod resample_sop;
//
mod reverse_sop;
//
mod rop_geometry_sop;
//
mod scatter_sop;
//
mod smooth_sop;
//
mod sort_sop;
//
mod sphere_sop;
//
mod subdivide_sop;
//
mod switch_sop;
//
mod transform_sop;
//
mod uvlayout_sop;
//
mod uvproject_sop;
//
mod uvunwrap_sop;
//
mod wrangle_sop;

/// One `r.register("<type>", <mod>::spec, <mod>::new);` per SOP.
pub(crate) fn builtins(r: &mut Registry) {
    r.register("null", null_sop::spec, null_sop::new);
    //
    r.register("add", add_sop::spec, add_sop::new);
    //
    r.register("attribcopy", attribcopy_sop::spec, attribcopy_sop::new);
    //
    r.register("attribcreate", attribcreate_sop::spec, attribcreate_sop::new);
    //
    r.register("attribdelete", attribdelete_sop::spec, attribdelete_sop::new);
    //
    r.register("attribnoise", attribnoise_sop::spec, attribnoise_sop::new);
    //
    r.register("attribpromote", attribpromote_sop::spec, attribpromote_sop::new);
    //
    r.register("attribtransfer", attribtransfer_sop::spec, attribtransfer_sop::new);
    //
    r.register("bend", bend_sop::bend_spec, bend_sop::bend_new);
    //
    r.register("blast", blast_sop::spec, blast_sop::new);
    //
    r.register("block_begin", block_sop::begin_spec, block_sop::begin_new);
    //
    r.register("block_end", block_sop::end_spec, block_sop::end_new);
    r.register("boolean", boolean_sop::spec, boolean_sop::new);
    //
    r.register("box", box_sop::spec, box_sop::new);
    //
    r.register("circle", circle_sop::spec, circle_sop::new);
    //
    r.register("clip", clip_sop::spec, clip_sop::new);
    //
    r.register("color", color_sop::spec, color_sop::new);
    //
    r.register("connectivity", connectivity_sop::spec, connectivity_sop::new);
    //
    r.register("copytopoints", copytopoints_sop::spec, copytopoints_sop::new);
    //
    r.register("curve", curve_sop::spec, curve_sop::new);
    //
    r.register("delete", delete_sop::spec, delete_sop::new);
    //
    r.register("divide", divide_sop::spec, divide_sop::new);
    //
    r.register("fuse", fuse_sop::spec, fuse_sop::new);
    //
    r.register("facet", facet_sop::spec, facet_sop::new);
    //
    r.register("file", file_sop::spec, file_sop::new);
    //
    r.register("grid", grid_sop::spec, grid_sop::new);
    //
    r.register("group", group_sop::spec, group_sop::new);
    //
    r.register("groupdelete", groupdelete_sop::spec, groupdelete_sop::new);
    //
    r.register("grouppromote", grouppromote_sop::spec, grouppromote_sop::new);
    //
    r.register("knife", knife_sop::spec, knife_sop::new);
    //
    r.register("line", line_sop::spec, line_sop::new);
    //
    r.register("matchsize", matchsize_sop::spec, matchsize_sop::new);
    //
    r.register("measure", measure_sop::spec, measure_sop::new);
    //
    r.register("merge", merge_sop::spec, merge_sop::new);
    //
    r.register("mirror", mirror_sop::spec, mirror_sop::new);
    //
    r.register("mountain", mountain_sop::spec, mountain_sop::new);
    //
    r.register("normal", normal_sop::spec, normal_sop::new);
    //
    r.register("output", output_sop::spec, output_sop::new);
    //
    r.register("peak", peak_sop::spec, peak_sop::new);
    //
    r.register("polybevel", polybevel_sop::spec, polybevel_sop::new);
    //
    r.register("polyextrude", polyextrude_sop::spec, polyextrude_sop::new);
    //
    r.register("ray", ray_sop::spec, ray_sop::new);
    //
    r.register("resample", resample_sop::spec, resample_sop::new);
    //
    r.register("reverse", reverse_sop::spec, reverse_sop::new);
    //
    r.register("rop_geometry", rop_geometry_sop::spec, rop_geometry_sop::new);
    //
    r.register("scatter", scatter_sop::spec, scatter_sop::new);
    //
    r.register("smooth", smooth_sop::spec, smooth_sop::new);
    //
    r.register("sort", sort_sop::spec, sort_sop::new);
    //
    r.register("sphere", sphere_sop::spec, sphere_sop::new);
    //
    r.register("subdivide", subdivide_sop::spec, subdivide_sop::new);
    //
    r.register("switch", switch_sop::spec, switch_sop::new);
    //
    r.register("twist", bend_sop::twist_spec, bend_sop::twist_new);
    //
    r.register("xform", transform_sop::spec, transform_sop::new);
    //
    r.register("uvlayout", uvlayout_sop::spec, uvlayout_sop::new);
    //
    r.register("uvproject", uvproject_sop::spec, uvproject_sop::new);
    //
    r.register("uvunwrap", uvunwrap_sop::spec, uvunwrap_sop::new);
    //
    r.register("wrangle", wrangle_sop::spec, wrangle_sop::new);
}

#[cfg(test)]
pub(crate) use null_sop::{new as null_new_for_tests, spec as null_spec_for_tests};
