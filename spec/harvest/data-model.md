# data-model: Houdini 20.5 geometry data model (HDK headers)  [H]

Source: `$H/toolkit/include/` {`GA/GA_Types.h`, `GA_Detail.h`, `GA_Attribute.h`, `GA_AttributeType.h`, `GA_Primitive.h`, `GA_ElementGroup.h`, `GA_Group.h`, `GA_ATIGroupBool.h`, `GA_IndexMap.h`, `GA_PrimitiveTypes.h`, `GA_PrimitiveFamilyMask.h`, `GA_Names.h`, `GEO/GEO_PrimType.h`, `GEO_PrimTypeCompat.h`, `GEO_PrimPoly.h`, `GU/GU_PrimPacked.h`}. `$H`=`/Applications/Houdini/Current/Frameworks/Houdini.framework/Versions/Current/Resources`. `GA_AttributeOwner.h` does NOT exist: the enum lives in `GA_Types.h:35`.
Tags: [H] = read from header (enum ordinals counted from source order); [R] = recalled, not in headers.

## 1 Owners (`GA_AttributeOwner`, GA_Types.h:35)
| id | enum | meaning |
|---|---|---|
| 0 | GA_ATTRIB_VERTEX | unique per-vertex (a prim corner) |
| 1 | GA_ATTRIB_POINT | shared; many vertices may reference one point |
| 2 | GA_ATTRIB_PRIMITIVE | one per prim |
| 3 | GA_ATTRIB_GLOBAL (= GA_ATTRIB_DETAIL alias) | single value on the detail |
| 4 | GA_ATTRIB_OWNER_N | sentinel; GA_ATTRIB_INVALID = -1 |
- Enum order is VERTEX,POINT,PRIM,GLOBAL (vertex first). VEX/UI order (point,vertex,prim,detail) is a different thing [R].

## 2 Attribute storage
`GA_Storage` (GA_Types.h:51): INVALID=-1, BOOL=0, UINT8=1, INT8=2, INT16=3, INT32=4, INT64=5, REAL16=6, REAL32=7, REAL64=8, STRING=9, DICT=10 [H].
`GA_StorageClass` (:73): INVALID=-1, INT=0, REAL(=FLOAT)=1, STRING=2, DICT=3, OTHER=4 [H]. Class decides which AIF (tuple / string-tuple / ...) an attribute supports.
- Tuple size: `GA_Attribute::getTupleSize()` (int, per-attribute, fixed at creation) [H]; owner `getOwner()`, class `getStorageClass()`, `getTypeInfo()` [H]. Tuple sizes of the usual names (P=3, N=3, Cd=3, uv=3, orient=4, pscale=1, id=1, up=3) are [R]; header only lists names (GA_Names.h: P, Pw, N, uv, v, w, Cd, Alpha, com, dist, lod, rest, mass, drag, tension, springk, life, age, up, orient, tangentu, tangentv, id, pscale, creaseweight, area, perimeter, width, instancefile, accel, layermerge, segs, div, …).
- Attribute *kinds* by `GA_AttributeType::findType(name)` registry (numeric, string, dict, …; names [R]).
- Paging: `GA_PAGE_BITS=10` → 1024-element pages (GA_Types.h:224) [H].

## 3 Type-info (`GA_TypeInfo`, GA_Types.h:101) — semantic tag on numeric attrs, drives transform behaviour
| id | enum | token |
|---|---|---|
| -1 | VOID | (none; no transform semantics) |
| 0 | POINT | "point" (position: affected by translate) |
| 1 | HPOINT | "hpoint" (position + homogeneous w; xyz NOT premultiplied) |
| 2 | VECTOR | "vector" (direction: rotate/scale, no translate) |
| 3 | NORMAL | "normal" (inverse-transpose) |
| 4 | COLOR | "color" |
| 5 | TRANSFORM | "matrix" |
| 6 | QUATERNION | "quaternion" |
| 7 | INDEXPAIR | "indexpair" |
| 8 | NONARITHMETIC_INTEGER | default for int-storage numeric attrs: never averaged/interpolated |
| 9 | ARITHMETIC_INTEGER | integers that ARE averaged/modified by arithmetic |
| 10 | TEXTURE_COORD | uv/uvw (not prim uv) |
- `GAisTransformingType(typeinfo)` + float class decide whether a Transform SOP touches an attr (GA_Attribute.h:439–441) [H].
- `GA_AttributeScope`: PUBLIC=0, PRIVATE=1, GROUP=2 (own namespace each; PRIVATE/GROUP not exported on save/merge by default) [H].

## 4 Groups
- `GA_GroupType` (GA_Types.h:161): POINT=0, PRIMITIVE=1, EDGE=2, BREAKPOINT=3, VERTEX=4, N=5; `GA_GroupMaskType` bit i = 1<<GA_GroupType [H]. Note POINT=0/PRIM=1 vs owner enum (VERTEX=0, POINT=1): different enums, do not mix.
- Element groups (point/prim/vertex): `GA_ElementGroup : GA_Attribute + GA_Group` (GA_ATIGroupBool.h:54) → a group IS a bool-ish attribute (scope GROUP) indexed by offset; classes `GA_PointGroup`, `GA_PrimitiveGroup`, `GA_VertexGroup` (GA_ElementGroup.h:33/74/168) [H].
- Ordered vs unordered: `GA_Group::isOrdered()` pure virtual; element groups can toggle `setOrdered(bool)` / `makeOrdered()` (GA_ATIGroupBool.h:139–153). Unordered = membership bit only (iteration in detail index order). Ordered = additionally keeps insertion order (`GA_ElementGroupOrder`). Conversion to unordered fails if group has "mixed" entries [H].
- Edge groups `GA_EdgeGroup : GA_Group` (container of GA_Edge, NOT an attribute); breakpoint groups `GA_BreakpointGroup : GA_Group` (curve/surface knot points) [H].
- Groups live in group tables (`GA_ElementGroupTable`, per-owner; edge/breakpoint tables separate); `obegin()` iterates alphabetically [H].

## 5 Primitive type ids (`GA_PrimitiveTypes.h`) — FACTORY ids, stable ("must always add to end")
| id | GA_PRIM* | GEO name / note |
|---|---|---|
| 0 | NONE | |
| 1 | POLY | polygon AND polyline: same type; open/closed = per-prim closed flag (`GA_Detail::getPrimitiveClosedFlag`, GEO_PrimPoly::isClosed) [H] |
| 2 | NURBCURVE | |
| 3 | BEZCURVE | |
| 4 | MESH | rows×cols hull ("Mesh" prim) |
| 5 | NURBSURF | |
| 6 | BEZSURF | |
| 7 | PASTESURF | |
| 8 | CIRCLE | quadric |
| 9 | SPHERE | quadric |
| 10 | TUBE | quadric |
| 11 | PART | particle prim |
| 12–16 | METABALL, METASQUAD, METALINE, METABEZ, METATRI | meta prims |
| 17 | TRIFAN | |
| 18 | TRISTRIP | |
| 19 | TRIBEZIER | |
| 20 | VOLUME | |
| 21 | TETRAHEDRON | volume element |
| 22 | POLYSOUP | |
| 23 | VDB | |
| 24 | HEXAHEDRON | volume element (Box SOP type=Primitive) |
| 25 | CHANNEL | |
| 26 | INTERNALSENTINEL | user/registered types get ids ≥ this |
- **Packed prims have NO factory id**: `GU_PrimPacked::isPackedPrimitive(typeId)` — registered at runtime (packed geo, packed disk, packed fragment, … `GU_Packed*.h`) → runtime ids [H]. Id is exposed via intrinsics (so never renumber).
- `GA_PrimitiveTypeId`: int wrapper, invalid = -1, `isValid()` = id≥0 [H]. Legacy bitmask API `GEO_PrimTypeCompat::GEOPRIMPOLY` etc. (`GA_PrimCompat::TypeMask`) = obsolete bitfield, not ids [H].
- Family masks (`GA_PrimitiveFamilyMask.h`): FACE=0x1, CURVE=0x2, HULL=0x4, TPSURF=0x8, QUADRIC=0x10, META=0x20, VOLUMEELEMENT=0x40; SPLINE=CURVE|TPSURF [H].
- Max NURBS/Bezier order `GA_MAX_ORDER=11` (matches Box/Grid/Sphere `orderrate` range `2!..11!` in DS) [H].

## 6 Offsets vs indices (`GA_Offset`, `GA_Index`; GA_IndexMap.h)
- Both are `GA_Size`=exint (64-bit) ordinals (GA_Types.h:236); in strict mode they are distinct types that don't convert implicitly [H].
- **Offset** = fixed data-slot handle into attribute arrays/groups/prim lists. Stable while elements are added/removed; deletions leave *vacancies* that later appends re-fill (so offset order ≠ creation order after edits). `GA_INVALID_OFFSET = -1`.
- **Index** = ordinal position in the detail's *ordered* element list (append order / `reorderPoint(offset,new_order)`); changes when anything before it is deleted/reordered. `GA_INVALID_INDEX = -1`.
- Why both: offsets give O(1) stable attribute access + cheap delete (no shifting arrays); indices give the user-visible, deterministic ordering (`@ptnum`, `@primnum`, point numbers in saved files). Conversions per owner: `pointOffset(idx)`/`pointIndex(off)`, `primitiveOffset/Index`, `vertexOffset/Index` (GA_Detail.h:343–520) via `GA_IndexMap::offsetFromIndex/indexFromOffset`. `getNumPoints() <= getNumPointOffsets()` always (vacancies) [H]. `forEachOffset`/ranges iterate offsets; `defragment` compacts so offset→index becomes identity [R].
- Detail-level attribs have no offset (single element).

## 7 Topology (vertex indirection)
- Prim holds an ordered vertex list (`getVertexCount/getVertexOffset(i)`); `vertexPoint(vtx)` → point it references; `vertexPrimitive(vtx)` → owning prim; `pointVertex(pt)` returns *a* (one) referencing vertex, GA_INVALID_OFFSET if point unreferenced (isolated points allowed; many vertices may share a point) (GA_Detail.h:529–560) [H].
- Hence point attr = shared across prims; vertex attr (e.g. `uv`, hard `N`) can differ per corner [H/R].

## 8 What `P` is
- `GA_Detail::getP()` returns `myP` — a first-class **point-owner** attribute named `P` (`GA_Names::P`); `getPos3(ptoff)` reads xyz, `getPw(ptoff)` reads `Pw` from a *separate* `Pw` attribute (rational NURBS/Bezier only, `getPwAttribute()`), `getPos4` assembles (xyz,w) [H].
- Storage chosen at detail construction: `GA_Detail(factory, GA_Storage Pstorage = GA_STORE_REAL32, ...)` (GA_Detail.h:111) → default **32-bit float** P (REAL64 allowed) [H]. Tuple 3 + type-info POINT [R].
- Not an arbitrary user attr: always exists, lifetime = detail's (cannot be deleted) [R].

## 9 Class stack (HDK)
`GA_Detail` (attribute tables, index maps, prim list, groups) → `GEO_Detail` (geometric ops, prim classes `GEO_Prim*`) → `GU_Detail` (high-level modelling utils; what SOPs hold as `gdp`) [H: file layout; roles R].
