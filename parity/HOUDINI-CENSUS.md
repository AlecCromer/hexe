# Houdini SOP coverage census (h36)

Source: `hou.sopNodeTypeCategory().nodeTypes()` in hython 20.5.487 (script: `oracle/houdini/census.py`). Names namespace-stripped (`kinefx::`, `apex::`, `beta::`, `Sop/ns::`), versions collapsed to latest. Hexe side: `hexe-cli nodes`.

## h36 implemented (this branch)

Six-SOP target, 4 shipped in the 90 min budget (all hython-twin MATCH, bit-for-bit through `houdini_twin`, ledger rows `sop-bound|torus|tube|platonic`):

| sop | fixtures | verdict | gaps |
|---|---|---|---|
| bound | sop-bound, -pad, -keep | MATCH x3 | boundtype sphere/rectangle, dodivs, oriented bbox, breakpoints/edges group types |
| torus | sop-torus, -xform, -alt | MATCH x3 | type mesh/nurbs/bezier/polysoup (no Mesh prim) |
| tube | sop-tube, -xform, -cap | MATCH x3 (type=1) | Houdini default type=prim (Tube prim xform not probed) -> hexe default poly; vertexnormals |
| platonic | sop-platonic (+cube/octa/icosa/dodeca/xform) | MATCH x6 | type 5 soccer ball |

Candidate list check: polyextrude, resample, fuse, clip, ray, peak, sort, normal, attribpromote, blast, delete, matchsize, knife, mirror, twist, subdivide, polyreduce, polybevel were ALREADY in hexe; `carve` and `lattice` were the only truly missing ones from the user list.

## Left (next picks, by Houdini docs popularity)

carve, lattice, polyframe, dissolve, clean, polysplit, edgedivide, split, trail, attribrandomize, attribswap, groupcreate/groupexpand/groupinvert, name, pointjitter, align, bulge, tube type=prim, torus/tube Mesh prim.

Method notes: classification into groups is keyword/list based (one-off script, not committed - AGENTS section 2); the raw hython dump is `oracle/houdini/census-houdini.json`; regenerate with `hython oracle/houdini/census.py`. `hidden()` types are counted as deprecated/legacy (152); `group`, `knife`, `twist` are hidden in 20.5 but hexe implements them.

## Numbers

| metric | n |
|---|---|
| raw `nodeTypes()` keys (all versions/namespaces) | 1127 |
| distinct SOP names | 1025 |
| hidden / legacy (Houdini `hidden()` flag = deprecated or internal) | 152 |
| visible (non-hidden) SOP names | 873 |
| hexe registered types | 68 |
| hexe covers visible Houdini SOPs | 64 (7.3% of visible) |
| hexe covers hidden/legacy names | 3 (group, knife, twist) |
| hexe-only / renamed | `wrangle` = Houdini `attribwrangle`; `motion` = hexe-only (no Houdini SOP) |

## By group (visible SOPs)

| group | houdini | hexe has | missing | note |
|---|---|---|---|---|
| core-modeling | 113 | 31 | 82 | target |
| attribute | 93 | 16 | 77 | target (incl. group*/uv*) |
| deform | 41 | 6 | 35 | target |
| scatter-copy | 33 | 2 | 31 | target (pack/unpack incl.) |
| io | 53 | 3 | 50 | file/import/export/ROP: partial skip |
| utility | 16 | 6 | 10 | subnet/switch/cache helpers |
| other | 5 | 0 | 5 | misc |
| sim-only | 519 | 0 | 519 | SKIP: solvers/crowds/vellum/rbd/VDB/volume/heightfield/kinefx rigging/etc. |
| deprecated (hidden) | 152 | 3 | 149 | legacy names, skip |

## Missing, by group

### core-modeling - 82 missing

cap carve circlefromedges circlespline clay clean cluster convexdecomposition crease crosssectionsurface
curveclay curvesect detangle dissolve edgecollapse edgecusp edgedivide edgeequalize edgeflip edgestraighten
edgetransport extractcentroid extrude fillet fit flatten font hole inflate intersectionanalysis
intersectionstitch isooffset join laplacian lsystem matchtopology metaball planarinflate planarpatch
planarpatchfromcurves planarpleat planartiles pointcloudiso pointcloudreduce pointcloudsurface polybridge
polycut polydoctor polyexpand2d polyhinge polyloft polypatch polypath polysoup polyspline polysplit polywire
primitive profile project quadremesh rails refine remesh remeshbubbles remeshgrid remeshtocamera rewire
shrinkwrap solidify spiral split sprite starburst stitch superquad surfsect trace triangulate2d tribez
tridivide trim

### attribute - 77 missing

attribadjustarray attribadjustcolor attribadjustdict attribadjustfloat attribadjustinteger attribadjustvector
attribblur attribcast attribcombine attribcomposite attribexpression attribfade attribfill attribfrommap
attribfromparm attribfrompieces attribfromvolume attribinterpolate attribmirror attribpaint attribrandomize
attribremap attribreorient attribstringedit attribswap attribtransferbyuv attribtransformcompute
attribtransformextract attribute attribvop computetransform distancealonggeometry distancefromgeometry
distancefromtarget enumerate error extracttransform groupbylasso groupcombine groupcopy groupcreate
groupexpand groupexpression groupfindpath groupfromattribboundary groupinvert grouppaint grouprange
grouprename groupsfromname grouptransfer heatgeodesic mappoints maskbyfeature measurethickness metagroups name
pointcloudmeasure pointcloudnormal polyframe proximity rest texture texturefeature uvautoseam uvbrush uvedit
uvflatten uvfuse uvpelt uvquickshade uvtransform verify_bsdf visibility visualize windingnumber xformbyattrib

### deform - 35 missing

align ballisticpath blendshapes bulge clothdeform comb copyxform creep deformationwrangle deltamush
dynamicwarp edit explodedview lattice latticefromvolume magnet matchaxis orientalongcurve pathdeform
pointdeform pointjitter projectilemotion relax ripple sblend sculpt shapediff softpeak softxform
stabilizejoint surfacedeform timeblend wrinkledeformer xformaxis xformpieces

### scatter-copy - 31 missing

clusterpoints copytocurves cregion extractpointfromcurve findinstances gluecluster instance mergepacked pack
packeddiskedit packededit packedfoldersplit packfolder packgroom packinject packpoints particletrail
pointgenerate pointreplicate pointsfromvolume pointweld repack scatteralign scatterintexturemask splitpoints
trail unpack unpackfolder unpackgroom unpackpoints unpackusd

### io - 50 missing

alembic alembicgroup alembicprimitive bakeode bakevolume characterio dopimport dopimportfield dopimportrecords
exportobjtransforms fbxanimimport fbxcharacterimport fbxskinimport filecache filemerge gltf lidarimport
lopimport mdd onnx opencl python rawimport rop_alembic rop_fbx rop_fbxanimoutput rop_fbxcharacteroutput
rop_geometryraw rop_gltf rop_gltfcharacteroutput scenecharacterexport scenecharacterimport script tableimport
testgeometry_capybara testgeometry_crag testgeometry_electra testgeometry_pighead testgeometry_rubbertoy
testgeometry_shaderball testgeometry_squab testgeometry_templatebody testgeometry_templatehead
testgeometry_tommy unix usdanimimport usdcharacterimport usdexport usdimport usdskinimport

### utility - 10 missing

cache cacheif compile_begin compile_end findshortestpath fractal mergegraph object_merge subnet switchif

### other - 5 missing

extrudevolume fluidcompress texturemaskpaint textureopticalflow usdconfigure

### sim-only / domain (skip) - 519

Not listed individually; prefixes: agent* crowd* flip* pyro* vellum* rbd* mpm* muscle* tissue* ocean* whitewater* particlefluid* feather* guide* hair* groom* rig* kinefx::* motionclip* mocap* character* heightfield* volume* vdb* cloud* sky* tet* fem* ragdoll* bone*/joint*/capture* cloth* pose* skeleton* wire* voronoi* dop*/lop*/top*/cop*/chop* nets. Full list: `oracle/houdini/census-houdini.json`.

### deprecated (hidden) - 152

agentfromrigcore attachcontrolgeo attribfind attribwranglecore autoflexlines bakevex bonesolidify break bridge
capturemeta capturepaintcore characterblendshapescore clipexport clipimport clustermesh
computemotionclipcreate computemotionclipretime cookie copy crowdmotionpathavoidcore crowdmotionpatheditcore
crowdmotionpathevaluatecore deform deformmeta deformmuscle delete_overlapping_polygons deterministicp2gcl
dopio drawhair duplicate elastictransform falloff fascia fasciasolidify fasciasolver featherattribinterpolate
featherconvert featherdeintersect_expandneighbors featherraycore feathertemplateinterpolatecore finalizewaves
flipmodifyvolume fluidsource foreach guidegroomcore guidemaskcore guideprocesscore hairclumpcore hairgencore
heightfield_maskbyconcavity heightfield_maskbyshadow iso kelvinletsculpt layer layoutbrushcomb
layoutbrushdelete layoutbrushfill layoutbrushline layoutbrushmove layoutbrushorient layoutbrushpaint
layoutbrushplace layoutbrushreplace layoutbrushscale layoutbrushstack loadslices mlexamplecreatecore
mlexampledecomposecore mlexampledeserializepacked mlexampledeserializepoint mlexampleserializepacked
mlexampleserializepoint mlextractexamplecore mlregressioninferencecore mlregressionproximitycore
motionclipcomputevelocity motionclipcreateclipinfo motionclipmerge motionclipunpack motionmixersetup muscle
musclecapture muscleconstraintproperties muscleconstraintpropertiesfem muscledeform muscleslideconstraint
musclesolver musclesolverfem muscletpose neighborsearchcl paint particle partition pendulummotioncore point
pointcapturebiharmonic pointcapturecore pointmap pointwrangle polycap polyknit polystitch poseweightinterp
primitivesplit projectilemotioncore ragdollconstraints rbdcustomfracture rbdrecursivefracture
rbdrewireconstraints resamplesplinetransforms rop_dembones_skinningconverter sceneaddconstraint
sceneconstraint shatter shop shotsculpt simplexrefine singlecurveoperation singlepointweld sky
slidemodifierpaint smoothmotioncore solidembed solidfracture splineik spring sssscatter strokecache
surfacerelax surfacesplat tetcraft tetlayer tetstrata timewarp tissuesolver topoflowbake tristrip
vdbcollidersprocess vdbcreatecl vectordeform vertex vertexsplit vex volumebin volumewranglecore vopsop
vopsoptype waveform wavevelocity
