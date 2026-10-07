# parms-merge: Merge SOP  [H] (negative result)

- No embedded dialog script found for `SOP_Merge` in any `libHoudini*.dylib` (scan: NUL-terminated `parm {` strings, all 21 libs) nor in `OPlibSop.hda` index (no `Sop/merge` entry) → type is C++-only; `SOP_MergeParms`/`SOP_MergeVerb` symbols exist (`libHoudiniOP2.dylib` strings, offset 41408376–41408403) → a generated parms struct exists but its template text was not recoverable by string scan. Parm list therefore **[?] for hidden/compiled parms**.
- `sop/merge.txt` (nodes.zip, 651 bytes) has **no `@parameters` section** → 0 documented UI parms. [H]
- Facts [H] from help: merges all inputs into one stream; max **9999** inputs; bypass exports only first input (does not compute merge); suggests Group SOP before merge to keep provenance.
- Source: `houdini/help/nodes.zip:sop/merge.txt`; symbols `libHoudiniOP2.dylib:41408376`, `:55008206` (`SOP_Merge.C` global-init symbol).
- harvested parm templates: **0** · help doc entries: **0**
- Open [?]: whether 20.5 Merge has any hidden/legacy parm (e.g. old `mergepacked`-style flags). `SOP_MergePacked` is a SEPARATE node (`mergepacked`, parms in `parms-*` not harvested: see INDEX).
