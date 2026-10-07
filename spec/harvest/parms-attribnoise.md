# parms-attribnoise: harvested Houdini 20.5 parameter templates  [H]

[H] = read from Houdini 20.5.487 on-disk install (no hython). Columns straight from embedded dialog-script (DS). `range` `N!` = hard-clamped bound (min/max locked); no `!` = UI slider hint only. `export all` = parm exported to VEX/ch(). menu `tok=Label`. `cond` = disablewhen/hidewhen.

## attribnoise::2.0  — attribnoise::2.0 HDA DialogScript
- source: `OPlibSop.hda:1197867 (len 45246)` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=0abf16aff2e7fac0 · 45246 bytes
- harvested parm templates: **82** · help doc entries (`sop/attribnoise.txt`): **38**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 | General | `group` | Group | string |  | 0 1 | (toggle)[ "hou.pwd().generateInputGroupMenu(0, parm=kwargs['parm'])" ]=language, python |  |  |
| 2 | General | `doblend` | Blend | toggle | off |  |  |  | joinnext, nolabel, tags:script_callback_language=python |
| 3 | General | `blendweight` | Blend | float | 1 | 0! 1! |  | disa: { doblend == 0 } { blendmode == attrib } | joinnext, tags:script_callback_language=python |
| 4 | General | `blendmode` | Scale Mode | ordinal | value |  | value=Constant, attrib=Use Attribute | disa: { doblend == 0 } | nolabel, tags:script_callback_language=python |
| 5 | General | `blendattrib` | Blend Attribute | string | mask |  |  | hide: { doblend == 0 } { blendmode == value } menureplace | tags:script_callback_language=python |
| 6 | General | `attribtype` | Attribute Names | ordinal | vec |  | float=Float          , vec=Vector |  | joinnext, tags:script_callback_language=python |
| 7 | General | `attribs` | Attribute Names | string | Cd | 0 1 | (toggle)[ "r = []" ]=[ "node = hou.pwd()" ], [ "inputs = node.inputs()" ]=[ "if inputs and inputs[0]:" ], [ "    geo = inputs[0].geometry()" ]=[ "    if geo:" ], [ "        c = node.parm('class').evalAsString()" ]=[ "        if c == 'detail':" ], [ "            attrs = geo.globalAttribs()" ]=[ "        elif c == 'primitive':" ], [ "            attrs = geo.primAttribs()" ]=[ "        elif c == 'point':" ], [ "            attrs = geo.pointAttribs()" ]=[ "        else: # vertex" ], [ "            attrs = geo.vertexAttribs()" ]=[ "            " ], [ "        attribtype = node.parm('attribtype').evalAsString() " ]=[ "        for a in attrs:" ], [ "            if 'float' in attribtype and a.dataType() == hou.attribData.Float and not a.isArrayType() and a.size() == 1:" ]=[ "                r.extend([a.name(), a.name()])" ], [ "            if 'vec' in attribtype and a.dataType() == hou.attribData.Float and not a.isArrayType() and a.size() == 3:" ]=[ "                r.extend([a.name(), a.name()])" ], [ "return r" ]=language, python |  | joinnext, nolabel |
| 8 | General | `componentscope` | Component Scope | buttonstrip | 111 |  | (toggle)x=X, y=Y, z=Z | disa: { attribtype != vec } | nolabel, tags:script_callback_language=python |
| 9 | General | `class` | Attribute Class | ordinal | point | 0 1 | point=Point, vertex=Vertex, primitive=Primitive , detail=Detail         |  |  |
| 10 | General | `displace` | Noise Along Vector | toggle | 0 |  |  | disa: { attribtype != vec } | tags:script_callback_language=python |
| 11 | General | `displaceattrib` | Vector Attribute | string | N | 0 1 |  | hide: { attribtype != vec } { displace != 1 } menureplace |  |
| 12 | Noise Value | `operation` | Operation | ordinal | add | 0 1 | set=Set, add=Add, sub=Subtract, mult=Multiply, min=Minimum, max=Maximum |  |  |
| 13 | Noise Value | `noiserange` | Range Values | ordinal | 0 |  | positive=Positive, zcentered=Zero Centered, custom=Min/Max, negative=Negative, minplusrange=Min + Range Length, midplusminusrange=Middle Â± Range Length |  | tags:script_callback_language=python |
| 14 | Noise Value | `amplitude` | Amplitude | float | 1 | 0! 10 |  | hide: { attribtype != float displace != 1 } { noiserange == custom } { noiserange == minplusrange } { noiserange == midplusminusrange } | tags:script_callback_language=python |
| 15 | Noise Value | `amplitudev` | Amplitude | float | 1 | 0! 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange == custom } { noiserange == minplusrange } { noiserange == midplusminusrange } | joinnext, tags:script_callback_language=python |
| 16 | Noise Value | `amplitudetype` | Amplitude Type | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control | hide: { attribtype != vec } { displace == 1 } { noiserange == custom } { noiserange == minplusrange } { noiserange == midplusminusrange } | nolabel, tags:script_callback=;script_callback_language=python |
| 17 | Noise Value | `amplitudescalev` | Amplitude Scale | vector[3] | 1 1 1 | 0! 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange == custom } { noiserange == minplusrange } { noiserange == midplusminusrange } { amplitudetype != | tags:script_callback_language=python |
| 18 | Noise Value | `rangemin` | Min Value | float | 0 | -10 10 |  | hide: { attribtype != float displace != 1 } { noiserange != custom noiserange != minplusrange } | tags:script_callback_language=python |
| 19 | Noise Value | `rangeminv` | Min Value | float | 0 | -10 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != custom noiserange != minplusrange } | joinnext, tags:script_callback_language=python |
| 20 | Noise Value | `rangemintype` | Range Min Type | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control | hide: { attribtype != vec } { displace == 1 } { noiserange != custom noiserange != minplusrange } | nolabel, tags:script_callback=;script_callback_language=python |
| 21 | Noise Value | `rangeminscalev` | Min Value Scale | vector[3] | 0 0 0 | -10 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != custom noiserange != minplusrange } { rangemintype != 1 } | tags:script_callback_language=python |
| 22 | Noise Value | `rangemax` | Max Value | float | 1 | -10 10 |  | hide: { attribtype != float displace != 1 } { noiserange != custom } | tags:script_callback_language=python |
| 23 | Noise Value | `rangemaxv` | Max Value | float | 1 | -10 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != custom } | joinnext, tags:script_callback_language=python |
| 24 | Noise Value | `rangemaxtype` | Range Max Type | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control | hide: { attribtype != vec } { displace == 1 } { noiserange != custom } | nolabel, tags:script_callback=;script_callback_language=python |
| 25 | Noise Value | `rangemaxscalev` | Max Value Scale | vector[3] | 1 1 1 | -10 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != custom } { rangemaxtype != 1 } | tags:script_callback_language=python |
| 26 | Noise Value | `midvalue` | Middle Value | float | 0 | 0 10 |  | hide: { attribtype != float displace != 1 } { noiserange != midplusminusrange } | tags:script_callback_language=python |
| 27 | Noise Value | `midvaluev` | Middle Value | float | 0 | 0 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != midplusminusrange } | joinnext, tags:script_callback_language=python |
| 28 | Noise Value | `midvaluetype` | Middle Value Type  | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control | hide: { attribtype != vec } { displace == 1 } { noiserange != midplusminusrange } | nolabel, tags:script_callback_language=python |
| 29 | Noise Value | `midvaluescalev` | Middle Value Scale | vector[3] | 1 1 1 | 0 10 |  | hide: { attribtype != vec } { displace == 1 } { midvaluetype != 1 } { noiserange != midplusminusrange } | tags:script_callback_language=python |
| 30 | Noise Value | `rangevalue` | Range Length | float | 1 | 0 10 |  | hide: { attribtype != float displace != 1 } { noiserange != minplusrange noiserange != midplusminusrange } | tags:script_callback_language=python |
| 31 | Noise Value | `rangevaluev` | Range Length | float | 1 | 0 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != minplusrange noiserange != midplusminusrange } | joinnext, tags:script_callback_language=python |
| 32 | Noise Value | `rangevaluetype` | Range Length Type | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control | hide: { attribtype != vec } { displace == 1 } { noiserange != minplusrange noiserange != midplusminusrange } | nolabel, tags:script_callback_language=python |
| 33 | Noise Value | `rangevaluescalev` | Range Length Scale | vector[3] | 1 1 1 | 0 10 |  | hide: { attribtype != vec } { displace == 1 } { noiserange != minplusrange noiserange != midplusminusrange } { rangevaluetype != 1 } | tags:script_callback_language=python |
| 34 | Noise Value | `outputraw` | Output Raw Value | toggle | on |  |  |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 35 | Noise Value | `enableremap` | Enable Remap Ramp | toggle | 0 |  |  |  | tags:script_callback_language=python |
| 36 | Noise Value | `remapramp` | Remap Ramp | ramp_flt | 2 | 1! 10 |  | hide: { enableremap != 1 } | tags:rampfloatdefault=1pos ( 0 ) 1value ( 0 ) 1interp ( linear ) 2pos ( 1 ) 2value (  |
| 37 | Noise Value/Value Correction | `dobias` | Do Bias | toggle | off |  |  |  | joinnext, nolabel, tags:parmvop=1;shaderparmcontexts=cvex |
| 38 | Noise Value/Value Correction | `bias` | Bias | float | 0.5 | 0! 1! |  | disa: { dobias != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 39 | Noise Value/Value Correction | `dogain` | Do Gain | toggle | off |  |  |  | joinnext, nolabel, tags:parmvop=1;shaderparmcontexts=cvex |
| 40 | Noise Value/Value Correction | `gain` | Gain | float | 0.5 | 0! 1! |  | disa: { dogain != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 41 | Noise Value/Value Correction | `dogamma` | Do Gamma | toggle | off |  |  |  | joinnext, nolabel, tags:parmvop=1;shaderparmcontexts=cvex |
| 42 | Noise Value/Value Correction | `gamma` | Gamma | float | 2.2 | 0! 5 |  | disa: { dogamma != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 43 | Noise Value/Value Correction | `docontrast` | Do Contrast | toggle | off |  |  |  | joinnext, nolabel, tags:parmvop=1;shaderparmcontexts=cvex |
| 44 | Noise Value/Value Correction | `contrast` | Contrast | float | 0 | 0! 1 |  | disa: { docontrast != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 45 | Noise Pattern | `basis` | Noise Type | ordinal | simplex | 0 1 | value_fast=Fast, sparse=Sparse Convolution, alligator=Alligator, perlin=Perlin, flow=Perlin Flow, simplex=Simplex, worleyFA=Worley Cellular F1, worleyFB=Worley Cellular F2-F1, mworleyFA=Manhattan Cellular F1, mworleyFB=Manhattan Cellular F2-F1, cworleyFA=Chebyshev Cellular F1, cworleyFB=Chebyshev Cellular F2-F1, pcloud=Perlin Cloud, scloud=Simplex Cloud, fscloud=Fast Simplex Cloud |  | tags:script_unquoted=1 |
| 46 | Noise Pattern | `locationattrib` | Location Attribute | string | P |  |  |  | tags:script_callback_language=python |
| 47 | Noise Pattern | `elementsize` | Element Size | float | 1 | 0! 10 |  |  | joinnext, tags:script_callback_language=python |
| 48 | Noise Pattern | `elementsizetype` | Element Size | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control |  | nolabel, tags:script_callback=;script_callback_language=python |
| 49 | Noise Pattern | `elementscale` | Element Scale | vector[3] | 1 1 1 | 0! 10 |  | hide: { elementsizetype != 1 } | tags:script_callback_language=python |
| 50 | Noise Pattern | `offset` | Offset | float | 0 | 0 50 |  |  | joinnext, tags:parmvop=1;shaderparmcontexts=cvex |
| 51 | Noise Pattern | `offsettype` | Offset | iconstrip | 0 |  | (toggle)BUTTONS_decompose_transforms=Per-Component Control |  | nolabel, tags:script_callback=;script_callback_language=python |
| 52 | Noise Pattern | `offsetv` | Offset | vector[3] | 0 0 0 | 0 50 |  | hide: { offsettype != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 53 | Noise Pattern | `usenoiseexpression` | Use VEXpression | toggle | off |  |  |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 54 | Noise Pattern | `noiseexpression` |   | string | // Parameters to modify:\n// Location Attribute - pos\n// Element Size - element |  |  | hide: { usenoiseexpression != 1 } menureplace | tags:editor=1;editorlang=vex;editorlines=8-40;script_callback_language=python |
| 55 | Noise Pattern | `vex_cwdpath` | Evaluation Node Path | oppath | . |  |  | hide: { usenoiseexpression != 1 } | tags:oprelative=.;script_callback_language=python |
| 56 | Noise Pattern/Animation | `animated` | Animate Noise | toggle | off |  |  | disa: { basis == value_fast } { basis == sparse } { basis == alligator } | tags:parmvop=1;shaderparmcontexts=cvex |
| 57 | Noise Pattern/Animation | `pulseduration` | Pulse Duration | float | 1 | 0! 10 |  | disa: { animated != 1 } { basis == value_fast } { basis == sparse } { basis == alligator } | tags:parmvop=1;shaderparmcontexts=cvex |
| 58 | Noise Pattern/Fractal | `fractal` | Fractal Type | ordinal | fBm | 0 1 | none=None, fBm=Standard (fBm), mfT=Terrain, hmfT=Hybrid Terrain | disa: { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fscloud } { attribtype == vec displace  | tags:script_callback=;script_callback_language=hscript;script_unquoted=1 |
| 59 | Noise Pattern/Fractal | `oct` | Max Octaves | float | 3 | 0! 16 |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud } | tags:parmvop=1;shaderparmcontexts=cvex |
| 60 | Noise Pattern/Fractal | `lac` | Lacunarity | float | 2.01234 | 0! 4 |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud } | tags:parmvop=1;shaderparmcontexts=cvex |
| 61 | Noise Pattern/Fractal | `rough` | Roughness | float | 0.5 | 0! 1! |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud } | tags:parmvop=1;shaderparmcontexts=cvex |
| 62 | Noise Pattern/Warping | `disp` | Lattice Warp | float | 0 | -1 1 |  | hide: { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fscloud } { attribtype == vec displace  | tags:parmvop=1;shaderparmcontexts=cvex |
| 63 | Noise Pattern/Warping | `dispesize` | Lattice Warp Size | float | 1 | 0 10 |  | hide: { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fscloud } { attribtype == vec displace  | tags:parmvop=1;shaderparmcontexts=cvex |
| 64 | Noise Pattern/Warping | `gflow` | Gradient Warp | float | 0 | -1 1 |  | hide: { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fscloud } { attribtype == vec displace  | tags:parmvop=1;shaderparmcontexts=cvex |
| 65 | Noise Pattern/Warping | `flowrot` | Flow Rotation | float | 0 | 0 1 |  | hide: { basis != flow basis != pflow } { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fsclou | tags:parmvop=1;shaderparmcontexts=cvex |
| 66 | Noise Pattern/Warping | `accuml` | Accumulate Lattice Warp | toggle | off |  |  | disa: { fractal == none }; hide: { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fscloud } { attribtype == vec displace  | tags:parmvop=1;shaderparmcontexts=cvex |
| 67 | Noise Pattern/Warping | `accumg` | Accumulate Gradient Warp | toggle | off |  |  | disa: { fractal == none }; hide: { attribtype == float basis == pcloud } { attribtype == float basis == scloud } { attribtype == float basis == fscloud } { attribtype == vec displace  | tags:parmvop=1;shaderparmcontexts=cvex |
| 68 | Noise Pattern/Warping | `distort` | Distortion | float | 1 | -1 1 |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud }; hide: { attribtype == float basis != pcloud basis != scloud basis != fscloud } { attribtype == vec displace == 0 } { attribtype == vec displace == 1 basis ! | tags:parmvop=1;shaderparmcontexts=cvex |
| 69 | Noise Pattern/Warping | `stretch` | Stretch | float[3] | 1 1 1 | 0! 10 |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud }; hide: { attribtype == float basis != pcloud basis != scloud basis != fscloud } { attribtype == vec displace == 0 } { attribtype == vec displace == 1 basis ! | tags:parmvop=1;shaderparmcontexts=cvex |
| 70 | Noise Pattern/Warping | `dodroop` | Do Droop | toggle | off |  |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud }; hide: { attribtype == float basis != pcloud basis != scloud basis != fscloud } { attribtype == vec displace == 0 } { attribtype == vec displace == 1 basis ! | joinnext, nolabel, tags:parmvop=1;shaderparmcontexts=cvex |
| 71 | Noise Pattern/Warping | `droop` | Droop | float | 1 | 0! 10 |  | disa: { dodroop != 1 } { fractal == none basis != pcloud basis != scloud basis != fscloud }; hide: { attribtype == float basis != pcloud basis != scloud basis != fscloud } { attribtype == vec displace == 0 } { attribtype == vec displace == 1 basis ! | tags:parmvop=1;shaderparmcontexts=cvex |
| 72 | Noise Pattern/Warping | `droopdir` | Droop Direction | float[3] | 1 0 0 | 0 10 |  | disa: { fractal == none basis != pcloud basis != scloud basis != fscloud }; hide: { dodroop != 1 } { attribtype == float basis != pcloud basis != scloud basis != fscloud } { attribtype == vec displace == 0 } { attribtype == vec disp | tags:parmvop=1;shaderparmcontexts=cvex |
| 73 | Noise Pattern/Worley Details | `doworleydetails` | Add Worley Details | toggle | off |  |  |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 74 | Noise Pattern/Worley Details | `worleyblend` | Blend | float | 1 | 0 1 |  | disa: { doworleydetails != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 75 | Noise Pattern/Worley Details | `worleyerosion` | Erosion | float | 0.5 | 0 1 |  | disa: { doworleydetails != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 76 | Noise Pattern/Worley Details | `worleyelementsizescale` | Element Size Scale | float | 2 | 0 5 |  | disa: { doworleydetails != 1 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 77 | Post-Process | `doclampmin` | Minimum | toggle | off |  |  |  | joinnext, nolabel, tags:script_callback_language=python |
| 78 | Post-Process | `clampminvalue` | Minimum | float | 0 | 0 10 |  | disa: { doclampmin == 0 } | tags:script_callback_language=python |
| 79 | Post-Process | `doclampmax` | Maximum | toggle | off |  |  |  | joinnext, nolabel, tags:script_callback_language=python |
| 80 | Post-Process | `clampmaxvalue` | Maximum | float | 1 | 0 10 |  | disa: { doclampmax == 0 } | tags:script_callback_language=python |
| 81 | Post-Process | `normalize` | Make Vectors Unit Length | toggle | 0 |  |  | hide: { attribtype != vec } | tags:script_callback_language=python |
| 82 | Post-Process | `updatenmls` | Recompute Normals | toggle | on |  |  | hide: { attribtype != vec } | tags:script_callback_language=python |

doc entries: Group; Blend; Blend Attribute; Attribute Type; Attribute Names; Component Scope; Attribute Class; Noise Along Vector; Vector Attribute; Range Values; Operation; Amplitude; Amplitude Scale; Min Value; Min Value Scale; Max Value; Max Value Scale; Middle Value; Middle Value Scale; Range Length; Range Length Scale; Output Raw Value; Enable Remap Ramp; Remap Ramp; Noise Type; Location Attribute; Element Size; Element Scale; Offset; Offset; Use VEXpression; VEX Expression; Enable Minimum; Minimum; Enable Maximum; Maximum; Make Vectors Unit Length; Recompute Normals

## attribnoise v1  — attribnoise (v1) HDA DialogScript
- source: `OPlibSop.hda:1137649 (len 14253)` (file:byte-offset of DS text, NUL-terminated) · DS sha256[:16]=d7246eae523f6569 · 14253 bytes
- harvested parm templates: **41** · help doc entries (`sop/attribnoise-.txt`): **27**

| # | folder | token | label | type[size] | default | range | menu | cond | cppname/flags |
|---|---|---|---|---|---|---|---|---|---|
| 1 |  | `group` | Group | string |  |  | (toggle)[script menu] |  |  |
| 2 |  | `grouptype` | Group Type | ordinal | guess |  | guess=Guess from Group, vertices=Vertices, edges=Edges, points=Points, prims=Primitives |  | tags:script_callback_language=python |
| 3 |  | `mode` | Mode | ordinal | add |  | add=Additive, mult=Multiplicative |  |  |
| 4 |  | `signature` | Signature | ordinal | dim3 |  | dim1=1D, dim3=3D |  |  |
| 5 |  | `locattrib` | Location Attribute | string | P |  |  |  |  |
| 6 |  | `attribs` | Attributes | string | Cd |  | (toggle)[ "r = []" ]=[ "node = hou.pwd()" ], [ "inputs = node.inputs()" ]=[ "if inputs and inputs[0]:" ], [ "    geo = inputs[0].geometry()" ]=[ "    if geo:" ], [ "        c = node.parm('class').evalAsString()" ]=[ "        if c == 'detail':" ], [ "            attrs = geo.globalAttribs()" ]=[ "        elif c == 'prim':" ], [ "            attrs = geo.primAttribs()" ]=[ "        elif c == 'point':" ], [ "            attrs = geo.pointAttribs()" ]=[ "        else: # vertex" ], [ "            attrs = geo.vertexAttribs()" ]=[ "        for a in attrs:" ], [ "            if a.dataType() == hou.attribData.Float and a.size() in [1, 3]:" ]=[ "                r.extend([a.name(), a.name()])" ], [ "return r" ]=language, python |  |  |
| 7 |  | `class` | Attribute Class | ordinal | point |  | detail=Detail, prim=Primitive, point=Point, vertex=Vertex |  | tags:script_callback_language=python |
| 8 |  | `sepparm` | Separator | separator |  |  |  |  |  |
| 9 |  | `animated` | Animated | toggle | 0 |  |  |  |  |
| 10 |  | `basis` | Noise Type | string | simplex |  | sine=Sinusoid, perlin=Perlin, pperlin=Periodic Perlin, simplex=Simplex, sparse=Sparse Convolution, flow=Perlin Flow, pflow=Periodic Perlin Flow, worleyFA=Worley Cellular F1, worleyFB=Worley Cellular F2-F1, mworleyFA=Manhattan Cellular F1, mworleyFB=Manhattan Cellular F2-F1, cworleyFA=Chebyshev Cellular F1, cworleyFB=Chebyshev Cellular F2-F1, alligator=Alligator |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 11 |  | `centernoise` | Center Noise | toggle | 0 |  |  | disa: { remap == 1 } |  |
| 12 |  | `amp1` | Amplitude | float | 1 | 0 10 |  | disa: { remap == 1 }; hide: { signature == dim3 } |  |
| 13 |  | `amp3` | Amplitude | vector[3] | 1 1 1 | -1 1 |  | disa: { remap == 1 }; hide: { signature == dim1 } |  |
| 14 |  | `remap` | Remap Noise | toggle | 0 |  |  |  |  |
| 15 |  | `outmin1` | Minimum | float | -1 | 0 10 |  | disa: { remap == 0 }; hide: { signature == dim3 } |  |
| 16 |  | `outmax1` | Maximum | float | 1 | 0 10 |  | disa: { remap == 0 }; hide: { signature == dim3 } |  |
| 17 |  | `outmin3` | Minimum | vector[3] | -1 -1 -1 | -1 1 |  | disa: { remap == 0 }; hide: { signature == dim1 } |  |
| 18 |  | `outmax3` | Maximum | vector[3] | 1 1 1 | -1 1 |  | disa: { remap == 0 }; hide: { signature == dim1 } |  |
| 19 |  | `pdf` | Distribution | ramp_flt | 2 | 1! 10 |  | disa: { remap == 0 } | tags:rampbasisdefault=monotonecubic;rampfloatdefault=1pos ( 0 ) 1value ( 0.5 ) 1inter |
| 20 |  | `pulseduration` | Pulse Duration | float | 1 | 0 10 |  | disa: { animated == 0 } |  |
| 21 |  | `tperiod` | Temporal Period | integer | 5 | 0 10 |  | disa: { animated == 0 } { basis != pperlin basis != pflow } |  |
| 22 |  | `elementsize` | Element Size | float | 1 | 0! 10 |  |  |  |
| 23 |  | `elementscale` | Element Scale | vector[3] | 1 1 1 | 0! 1 |  |  |  |
| 24 |  | `speriod` | Spatial Period | intvector[3] | 10 10 10 | -1 1 |  | disa: { basis != pperlin basis != pflow } |  |
| 25 |  | `offset` | Offset | vector[3] | 0 0 0 | -1 1 |  |  |  |
| 26 | Fractal | `fractal` | Fractal Type | string | fBm |  | none=None, fBm=Standard (fBm), mfT=Terrain, hmfT=Hybrid Terrain |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 27 | Fractal | `oct` | Max Octaves | float | 3 | 0 16 |  | hide: { fractal == none } | tags:parmvop=1;shaderparmcontexts=cvex |
| 28 | Fractal | `lac` | Lacunarity | float | 2.01234 | 0 4 |  | hide: { fractal == none } | tags:parmvop=1;shaderparmcontexts=cvex |
| 29 | Fractal | `rough` | Roughness | float | 0.5 | 0 1 |  | hide: { fractal == none } | tags:parmvop=1;shaderparmcontexts=cvex |
| 30 | Warping | `dolwarp` | Enable Lattice Warp | toggle | off |  |  |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 31 | Warping | `accuml` | Accumulate Lattice Warp | toggle | off |  |  | hide: { dolwarp == 0 } { fractal == none } | tags:parmvop=1;shaderparmcontexts=cvex |
| 32 | Warping | `disp` | Lattice Warp | float | 0 | -0.5 0.5 |  | hide: { dolwarp == 0 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 33 | Warping | `dispfreq` | Freq | float | 1 | 0 1 |  | hide: { dolwarp == 0 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 34 | Warping | `dogwarp` | Enable Gradient Warp | toggle | off |  |  |  | tags:parmvop=1;shaderparmcontexts=cvex |
| 35 | Warping | `accumg` | Accumulate Gradient Warp | toggle | off |  |  | hide: { dogwarp == 0 } { fractal == none } | tags:parmvop=1;shaderparmcontexts=cvex |
| 36 | Warping | `gflow` | Gradient Warp | float | 0 | -0.5 0.5 |  | hide: { dogwarp == 0 } | tags:parmvop=1;shaderparmcontexts=cvex |
| 37 | Warping | `flowrot` | Flow Rotation | float | 0 | 0 1 |  | hide: { basis != flow basis != pflow } | tags:parmvop=1;shaderparmcontexts=cvex |
| 38 | Post-Process | `clampmin` | Minimum | toggle | 0 |  |  |  | joinnext, nolabel |
| 39 | Post-Process | `minval` | Minimum | float | 0 | 0 10 |  | disa: { clampmin == 0 } |  |
| 40 | Post-Process | `clampmax` | Maximum | toggle | 0 |  |  |  | joinnext, nolabel |
| 41 | Post-Process | `maxval` | Maximum | float | 1 | 0 10 |  | disa: { clampmax == 0 } |  |

doc entries: Mode; Signature; Location Attribute; Attributes; Group; Animated; Noise Type; Center Noise; Amplitude; Remap Noise; Minimum; Maximum; Distribution; Pulse Duration; Temporal Period; Element Size; Element Scale; Spatial Period; Offset; Fractal Type; Max Octaves; Lacunarity; Roughness; Enable Lattice Warp; Enable Gradient Warp; Minimum; Maximum
