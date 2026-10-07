# cook-idiom: how a real Houdini SOP cooks (HDK samples)  [H]

Read: `$H/toolkit/samples/SOP/SOP_Star/SOP_Star.C` (verb API, generator, 292 l), `SOP_PointWave.C` (legacy `cookMySop`, modifier, 170 l), `SOP_Flatten.C` (legacy, local vars, 398 l), `SOP_WindingNumber/*.C`, `SOP_CopyToPoints/SOP_CopyToPointsHDK.C`, `SOP_Sweep/*`; headers `SOP/SOP_NodeVerb.h`, `SOP/SOP_Node.h`, `SOP/SOP_Error.h`. `$H`=`/Applications/Houdini/Current/Frameworks/Houdini.framework/Versions/Current/Resources`. HDK sample licence = BSD-style redistribution OK; we cite structure only.

## 1 Two generations of API (both live in 20.5)
| gen | entry | used by | notes |
|---|---|---|---|
| legacy | `SOP_Node::cookMySop(OP_Context&)` | PointWave, Flatten, DualStar | node object owns state; parms via `evalFloat("name", vecidx, time)` |
| verb (modern, threadable/compiled-SOP) | `SOP_NodeVerb::cook(const CookParms&)` | Star, WindingNumber, Sweep, CopyToPoints | stateless; typed parms struct `SOP_<X>Parms` auto-generated from the DS (`SOP_X.proto.h`); legacy node just forwards: `cookMySop(ctx){ return cookMyselfAsVerb(ctx); }` |
Built-ins (Box/Grid/Sphere/…) are verb SOPs whose DS is embedded in the dylib → same shape as `SOP_Star` (`theDsFile` raw string + `PRM_TemplateBuilder`; see `parms-*.md` for real ones) [H].

## 2 Registration
`void newSopOperator(OP_OperatorTable*)` = dll hook: `table->addOperator(new OP_Operator(internalName, uiName, ctor, templates, minInputs, maxInputs, localVars, flags))`. Star: 0 min/0 max inputs + `OP_FLAG_GENERATOR`; PointWave: min 1 / max 1, no flags. Internal type name must be globally unique (e.g. `hdk_star`). Verb registers via static `SOP_NodeVerb::Register<Verb>` and `cookVerb()` returns it [H].

## 3 Cook modes (`SOP_NodeVerb::CookMode`, SOP_NodeVerb.h:158) — how the output detail (`gdh()`) is initialised
| mode | dest gdp at cook start | inputs |
|---|---|---|
| COOK_INPLACE | first input stolen, or a COPY of it | input 0 = null (others valid) |
| COOK_DUPLICATE | copy of input 0 | all valid |
| COOK_GENERATOR | empty (maybe `stashAll()`) | all valid |
| COOK_INSTANCE | dest gdp INVALID | all valid |
| COOK_GENERIC | unspecified: may hold the **previous cook's result** (or be empty after cache eviction) | all valid |
| COOK_PASSTHROUGH | cook() never called; input 0 passed through (empty gdp if no input 0) | — |
WindingNumber comment: INPLACE chosen when SOP always duplicates input 0 anyway and gains nothing from the previous result; GENERIC lets output detail be separate from the input and survive across cooks [H].

## 4 Input locking + access
- Legacy: first line of `cookMySop` = `OP_AutoLockInputs inputs(this); if (inputs.lock(context) >= UT_ERROR_ABORT) return error();` — RAII unlock; **never call `unlockInputs` by hand** (PointWave.C/Flatten.C comments) [H]. Inputs cook on demand at lock.
- Verb: `cookparms.hasInput(i)`, `cookparms.inputGeo(i)` → `const GU_Detail*` (null if absent), `inputGeoHandle(i)`; output `cookparms.gdh().gdpNC()` → `GU_Detail*` (non-const) [H].
- Min/max input counts enforced at registration, `-1`/absent inputs handled by `hasInput` [H].

## 5 gdp duplicate vs clear
- Legacy modifiers: `duplicateSource(0, context)` copies input 0 into `gdp` (members: `gdp`, `myGdpHandle`). `duplicatePointSource(0, context)` = same with hint "only points (P/point attrs) altered" → cheap rollback when input unchanged (PointWave) [H].
- Generators (Star): `detail->clearAndDestroy()` wipes everything "except the empty P and topology attributes"; first cook / cache-evicted / changed size → rebuild, else **only rewrite P** (COOK_GENERIC reuse trick: if `getNumPoints()==npoints` keep topology, `getP()->bumpDataId()`) [H].
- Build idiom (Star): `appendPrimitivesAndVertices(GA_PRIMPOLY, nprims, nverts_per, start_vtxoff, closed=true)` → contiguous vertex block; `appendPointBlock(n)` → contiguous point offsets; wire `setVertexPoint(vtx0+i, pt0+i)`; then `bumpDataIdsForAddOrRemove(true,true,true)`; write `setPos3(ptoff, pos)`; contiguous blocks ⇒ `ptoff = start + i` valid [H]. Closed flag distinguishes polygon vs polyline (same GA_PRIMPOLY id) [H].

## 6 Parm eval at time
- Legacy: `fpreal t = context.getTime();` then `evalFloat("amp", 0, t)`/`evalInt(idx,vec,t)`; evaluate OUTSIDE the point loop (perf) unless local variables ($CR …) force per-element eval [H]. `SOP_CopyToPointsHDK` uses `evalInt(1,0,CHgetEvalTime())` for group-type [H].
- Verb: `auto&& p = cookparms.parms<SOP_XParms>();` typed getters (`getDivs()`, `getRad().y()`, `getOrient()` enum ‘SOP_StarParms::Orient::XY’, `getNradius()`); eval time = `cookparms.getCookTime()` (parms are already evaluated at that time) [H].
- Local variables: `setVariableOrder(3,2,0,1)` (sets which attr class binds first for $VAR names; per sample comment point attrs take precedence), `setCurGdh(0, myGdpHandle)`, `setupLocalVars()`; override `evalVariableValue` must still call base [H].

## 7 Groups
Legacy: `cookInputGroups(context, alone)` calls `cookInputPointGroups(context, myGroup, alone, setSel, groupParmIdx, grpTypeParmIdx, allowExisting, ordered, detached, inputIdx)`; null group ⇒ whole geometry; iterate with `GA_FOR_ALL_GROUP_PTOFF(gdp, myGroup, ptoff)` or `GA_Iterator it(gdp->getPointRange(myGroup)); it.blockAdvance(start,end)`. Empty group ⇒ skip work (Flatten: `!myGroup || !myGroup->isEmpty()`). `cookInputAllGroups(...)` + `GA_GROUP_POINT/PRIMITIVE` when a SOP has a Group Type menu (CopyToPoints) [H]. Menu entry `guess` = infer type from group string (see parms-wrangle.md) [H].

## 8 Errors / warnings / interrupt
- Legacy: `addError(SOP_MESSAGE, "text")`, `addWarning(code, msg)`, `addMessage(code,msg)`; status returned is `error()` (`OP_ERROR` ≈ `UT_ERROR_*`); abort threshold: `error() >= UT_ERROR_ABORT` ⇒ stop and `return error()` [H]. Codes (`SOP_Error.h`): `SOP_ERR_INVALID_SRC=0` ("no source or source has error"), `SOP_ERR_FILEGEO`, `SOP_ERR_BADGROUP`, `SOP_ERR_BAD_POINTGROUP`, `SOP_MESSAGE` (free text), `SOP_ERR_ILLEGAL_GROUPNAME` … [H].
- Verb: `cookparms.sopAddWarning(SOP_MESSAGE, "…")`, `cookparms.sopAddError(SOP_MESSAGE, "At least one input is required…")`, group-bad: `sopAddWarning(SOP_ERR_BADGROUP, name)` [H]. Warning = result still produced; Error = cook fails (node red).
- Interrupt: `UT_AutoInterrupt boss("msg"); if (boss.wasInterrupted()) return/break;` polled in the point loop [H].

## 9 Cook flags / data-id discipline
- `mySopFlags.setManagesDataIDs(true)` (ctor): SOP promises to bump the data ID of every attribute it changes; default (false) ⇒ Houdini bumps ALL ids after cook ("everything may have changed"). If you opt in and forget a bump, viewport/downstream caches go stale [H].
- Bump API: `attr->bumpDataId()`; `detail->bumpDataIdsForAddOrRemove(points,verts?,prims?)`; PointWave bumps P only if group non-empty [H].
- OP_FLAG_GENERATOR on registration = no required input / appears in generator context [H]. Other flags: `setNeedGuide*`/cookMyGuide1 for guide geo (Flatten.cookMyGuide1 builds bbox guide from `inputGeo(0)->getBBox`) [H].

## 10 Hexe port consequences [R, for spec authors]
Cook = pure fn(parms@time, inputs[]) → detail. Keep (a) ordered contiguous blocks for generators (point order observable via @ptnum), (b) closed-flag on polys, (c) group-null = all, (d) warnings vs errors distinction, (e) per-attribute change counter (data id) as the cache-invalidation primitive.
