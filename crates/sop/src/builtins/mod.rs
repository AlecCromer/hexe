//! THE merge point for SOP lanes. A lane makes `builtins/<name>_sop.rs` and touches exactly TWO lines here: its `mod` slot
//! and its `register` slot. Slots are pre-seeded, alphabetical, separated by `//` lines so parallel lanes never
//! produce adjacent hunks: uncomment YOUR two lines, nothing else. New type not listed → insert a line in its
//! sorted position between separators.
use crate::Registry;

mod null_sop;
//
// mod attribnoise_sop;
//
// mod box_sop;
//
// mod copytopoints_sop;
//
// mod grid_sop;
//
// mod line_sop;
//
// mod merge_sop;
//
// mod scatter_sop;
//
// mod sphere_sop;
//
mod transform_sop;
//
// mod wrangle_sop;

/// One `r.register("<type>", <mod>::spec, <mod>::new);` per SOP.
pub(crate) fn builtins(r: &mut Registry) {
    r.register("null", null_sop::spec, null_sop::new);
    //
    // r.register("attribnoise", attribnoise_sop::spec, attribnoise_sop::new);
    //
    // r.register("box", box_sop::spec, box_sop::new);
    //
    // r.register("copytopoints", copytopoints_sop::spec, copytopoints_sop::new);
    //
    // r.register("grid", grid_sop::spec, grid_sop::new);
    //
    // r.register("line", line_sop::spec, line_sop::new);
    //
    // r.register("merge", merge_sop::spec, merge_sop::new);
    //
    // r.register("scatter", scatter_sop::spec, scatter_sop::new);
    //
    // r.register("sphere", sphere_sop::spec, sphere_sop::new);
    //
    r.register("xform", transform_sop::spec, transform_sop::new);
    //
    // r.register("wrangle", wrangle_sop::spec, wrangle_sop::new);
}

#[cfg(test)]
pub(crate) use null_sop::{new as null_new_for_tests, spec as null_spec_for_tests};
