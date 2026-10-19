"""Synthetic Attribute Adjust Float / Integer / Color cases recorded from real Houdini (hython; oracle tooling, portable:
run as `hython sop_attribadjust_cases.py <out-dir>`). Writes:
  input.geo   the synthetic input (8x8 point grid, 49 quads) with point / prim / vertex / detail attributes and a point group;
  cases.json  one entry per case: node type, the parms set (flat Houdini names: menus as tokens, ramps / multiparms as
              their instance parms), the adjusted attribute (class, name) and Houdini's resulting values for it (or
              `"unchanged": true` when Houdini leaves the geometry alone).
Parm values are applied in order, so a ramp / multiparm count comes before its instance parms.
"""
import hou, json, os, re, sys

out = sys.argv[1]
os.makedirs(out, exist_ok=True)


def save(g, path):
    """saveToFile, then blank the machine / user fields Houdini stamps into `info` (fixtures must be portable)."""
    g.saveToFile(path)
    with open(path, encoding='utf-8') as f:
        text = f.read()
    for key in ('hostname', 'artist', 'date'):
        text = re.sub(r'"%s":"[^"]*"' % key, '"%s":""' % key, text)
    with open(path, 'w', encoding='utf-8', newline='\n') as f:
        f.write(text)


geo = hou.node('/obj').createNode('geo')
grid = geo.createNode('grid')
grid.parm('rows').set(8)
grid.parm('cols').set(8)
grid.parm('sizex').set(3.5)
grid.parm('sizey').set(2.5)
build = geo.createNode('python')
build.setInput(0, grid)
build.parm('python').set('''
node = hou.pwd(); g = node.geometry()
fr = lambda x: x - int(x)
for name, default in (('pscale', 0.0), ('mask', 0.0), ('w', 0.0)):
    g.addAttrib(hou.attribType.Point, name, default)
g.addAttrib(hou.attribType.Point, 'variant', 0)
g.addAttrib(hou.attribType.Point, 'id', 0)
g.addAttrib(hou.attribType.Point, 'Cd', (1.0, 1.0, 1.0))
g.addAttrib(hou.attribType.Point, 'iv', (0, 0))
grp = g.createPointGroup('grp')
for p in g.points():
    i = p.number()
    p.setAttribValue('pscale', fr(i * 0.37) * 2.0 - 0.5)
    p.setAttribValue('mask', (i % 9) / 6.0 - 0.25)
    p.setAttribValue('w', (i % 5) / 4.0)
    p.setAttribValue('variant', (i * 7) % 11 - 3)
    p.setAttribValue('id', 100 + 3 * i)
    p.setAttribValue('Cd', (fr(i * 0.13), fr(i * 0.29) - 0.1, fr(i * 0.71) * 1.2))
    p.setAttribValue('iv', (i, -i))
    if i % 3 == 0:
        grp.add(p)
g.addAttrib(hou.attribType.Prim, 'pscale', 0.0)
g.addAttrib(hou.attribType.Prim, 'variant', 0)
for pr in g.prims():
    pr.setAttribValue('pscale', pr.number() * 0.1)
    pr.setAttribValue('variant', pr.number() % 4)
g.addAttrib(hou.attribType.Global, 'dval', 2.5)
g.addAttrib(hou.attribType.Global, 'dint', 4)
vm = g.findGlobalAttrib('varmap')  # hscript variable map (string array): not part of the case
if vm is not None:
    vm.destroy()
''')
save(build.geometry(), os.path.join(out, 'input.geo'))

LIN = 'linear'
cases = [
    # ---------------- float ----------------
    ('f_const_add', 'attribadjustfloat', {'singlevalue': 0.3}),
    ('f_const_set_new_group', 'attribadjustfloat', {'group': 'grp', 'attrib': 'newf', 'operation': 'set', 'singlevalue': 1.5}),
    ('f_init_existing', 'attribadjustfloat', {'operation': 'init', 'singlevalue': 2.0}),
    ('f_init_new', 'attribadjustfloat', {'attrib': 'newf', 'operation': 'init', 'singlevalue': 2.0}),
    ('f_sub', 'attribadjustfloat', {'operation': 'sub', 'singlevalue': 0.25}),
    ('f_mult', 'attribadjustfloat', {'operation': 'mult', 'singlevalue': -1.75}),
    ('f_min', 'attribadjustfloat', {'operation': 'min', 'singlevalue': 0.25}),
    ('f_max', 'attribadjustfloat', {'operation': 'max', 'singlevalue': 0.25}),
    ('f_rand_set', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'minvalue': -1.0, 'maxvalue': 3.0}),
    ('f_rand_uniform_minplus', 'attribadjustfloat', {'valuetype': 'rand', 'distribinrange': 'uniform', 'randomseed': 7,
                                                     'rangemethod': 'minplusrange', 'minvalue': 2.0, 'rangevalue': 0.5}),
    ('f_rand_mid', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'rangemethod': 'midplusminusrange',
                                         'midvalue': 1.0, 'rangevalue': 4.0}),
    ('f_rand_ramp', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'remapramp': 3, 'remapramp2pos': 0.3,
                                          'remapramp2value': 0.8, 'remapramp2interp': LIN, 'remapramp3pos': 1.0,
                                          'remapramp3value': 1.0, 'remapramp3interp': LIN}),
    ('f_rand_ramp_const', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'remapramp': 3,
                                                'remapramp1interp': 'constant', 'remapramp2pos': 0.4, 'remapramp2value': 0.5,
                                                'remapramp2interp': 'constant', 'remapramp3pos': 1.0, 'remapramp3value': 1.0,
                                                'remapramp3interp': 'constant'}),
    ('f_rand_step', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'distribinrange': 'uniform',
                                          'enablestepping': 1, 'stepsize': 0.25, 'maxvalue': 2.0}),
    ('f_rand_outliers_both', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'distribinrange': 'outliers',
                                                   'outlierpercent': 30.0, 'outlierrange': 2.0, 'outlierside': 'both'}),
    ('f_rand_outliers_mult', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'distribinrange': 'outliers',
                                                   'outlierpercent': 40.0, 'outlierrange': 0.5, 'outlieroperation': 'mult',
                                                   'outlierside': 'above', 'outlierseed': 11, 'minvalue': 1.0, 'maxvalue': 5.0}),
    ('f_rand_custom_seed', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'randmethod': 'custom',
                                                 'randattrib': 'id', 'randomseed': 3}),
    ('f_rand_prim', 'attribadjustfloat', {'class': 'primitive', 'operation': 'add', 'valuetype': 'rand'}),
    ('f_rand_vertex_new', 'attribadjustfloat', {'class': 'vertex', 'attrib': 'vf', 'valuetype': 'rand'}),
    ('f_rand_detail', 'attribadjustfloat', {'class': 'detail', 'attrib': 'dval', 'operation': 'mult', 'valuetype': 'rand'}),
    ('f_line', 'attribadjustfloat', {'attrib': 'lin', 'operation': 'set', 'valuetype': 'line', 'distribinrange': 'uniform',
                                     'startposx': -1.0, 'endposx': 1.5, 'endposz': 1.0, 'maxvalue': 10.0}),
    ('f_radial', 'attribadjustfloat', {'attrib': 'rad', 'operation': 'set', 'valuetype': 'radial', 'distribinrange': 'uniform',
                                       'startposx': 0.5, 'startposz': 0.25, 'endposx': 1.5, 'endposz': 0.0}),
    ('f_bbox', 'attribadjustfloat', {'attrib': 'bb', 'operation': 'set', 'valuetype': 'bbox', 'bounddir': 'z'}),
    ('f_bbox_bounds', 'attribadjustfloat', {'attrib': 'bb', 'operation': 'set', 'valuetype': 'bbox', 'bounddir': 'x',
                                            'doboundmin': 1, 'boundmin': -0.5, 'doboundmax': 1, 'boundmax': 1.0}),
    ('f_remap_clamp', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'remapattrib', 'remapattrib_rangemax': 0.8,
                                            'minvalue': 1.0, 'maxvalue': 2.0}),
    ('f_remap_cycle', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'remapattrib', 'remapattrib_rangemin': 0.1,
                                            'remapattrib_rangemax': 0.5, 'remapattrib_rangemode': 'cycle'}),
    ('f_attrib', 'attribadjustfloat', {'valuetype': 'attrib', 'adjustmentattrib': 'mask', 'adjustmentattribscale': 2.0,
                                       'adjustmentattriboffset': 0.5}),
    ('f_specific', 'attribadjustfloat', {'operation': 'set', 'valuetype': 'rand', 'rangemethod': 'specific', 'nvalues': 3,
                                         'value0': 1.0, 'weight0': 1.0, 'value1': 5.0, 'weight1': 2.0, 'value2': 9.0,
                                         'weight2': 0.5}),
    ('f_blend', 'attribadjustfloat', {'operation': 'set', 'singlevalue': 4.0, 'doblend': 1, 'blendweight': 0.3}),
    ('f_blend_attrib', 'attribadjustfloat', {'operation': 'set', 'singlevalue': 4.0, 'doblend': 1, 'blendmode': 'attrib',
                                             'blendattrib': 'w'}),
    ('f_clamp', 'attribadjustfloat', {'valuetype': 'rand', 'enable_postprocess': 1, 'doclampmin': 1, 'clampminvalue': 0.0,
                                      'doclampmax': 1, 'clampmaxvalue': 0.6}),
    ('f_int_cast', 'attribadjustfloat', {'attrib': 'variant', 'singlevalue': 0.5}),
    ('f_wrong_type', 'attribadjustfloat', {'attrib': 'iv', 'singlevalue': 0.5}),
    ('f_default_new_group', 'attribadjustfloat', {'group': 'grp', 'attrib': 'newd', 'dodefault': 1, 'default': 7.0,
                                                  'singlevalue': 1.0}),
    # ---------------- integer ----------------
    ('i_const_add', 'attribadjustinteger', {'attrib': 'variant', 'singlevalue': 3}),
    ('i_rand_set', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand'}),
    ('i_rand_uniform', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                               'distribinrange': 'uniform', 'minvalue': -5, 'maxvalue': 20, 'randomseed': 3}),
    ('i_rand_mid', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                           'rangemethod': 'midplusminusrange', 'midvalue': 5, 'rangevalue': 3}),
    ('i_rand_mid_half', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                                'rangemethod': 'midplusminusrange', 'midvalue': 5, 'rangevalue': 5}),
    ('i_rand_mid_neg', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                               'rangemethod': 'midplusminusrange', 'midvalue': -5, 'rangevalue': 3}),
    ('i_rand_minplus', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'add', 'valuetype': 'rand',
                                               'rangemethod': 'minplusrange', 'minvalue': 2, 'rangevalue': 4}),
    ('i_rand_step', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                            'distribinrange': 'uniform', 'maxvalue': 30, 'enablestepping': 1, 'stepsize': 4}),
    ('i_rand_ramp', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand', 'remapramp': 3,
                                            'remapramp2pos': 0.5, 'remapramp2value': 0.2, 'remapramp2interp': LIN,
                                            'remapramp3pos': 1.0, 'remapramp3value': 1.0, 'remapramp3interp': LIN}),
    ('i_rand_outliers', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                                'distribinrange': 'outliers', 'outlierpercent': 40.0, 'outlierrange': 3.0,
                                                'outlierside': 'both'}),
    ('i_rand_outliers_mult', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                                     'distribinrange': 'outliers', 'outlierpercent': 50.0,
                                                     'outlierrange': 0.5, 'outlieroperation': 'mult', 'outlierside': 'below'}),
    ('i_rand_custom_seed', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                                   'randmethod': 'custom', 'randattrib': 'id'}),
    ('i_line', 'attribadjustinteger', {'attrib': 'lin', 'operation': 'set', 'valuetype': 'line', 'distribinrange': 'uniform',
                                       'startposx': -1.0, 'endposx': 1.5, 'endposz': 1.0}),
    ('i_attrib', 'attribadjustinteger', {'attrib': 'variant', 'valuetype': 'attrib', 'adjustmentattrib': 'mask',
                                         'adjustmentattribscale': 10.0}),
    ('i_specific', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'valuetype': 'rand',
                                           'rangemethod': 'specific', 'nvalues': 2, 'value0': 3, 'weight0': 1.0,
                                           'value1': 8, 'weight1': 3.0}),
    ('i_blend', 'attribadjustinteger', {'attrib': 'variant', 'operation': 'set', 'singlevalue': 10, 'doblend': 1,
                                        'blendweight': 0.5}),
    ('i_clamp', 'attribadjustinteger', {'attrib': 'variant', 'singlevalue': 2, 'enable_postprocess': 1, 'doclampmin': 1,
                                        'clampminvalue': 0, 'doclampmax': 1, 'clampmaxvalue': 5}),
    ('i_prim', 'attribadjustinteger', {'class': 'primitive', 'attrib': 'variant', 'operation': 'mult', 'singlevalue': 3}),
    ('i_float_attr', 'attribadjustinteger', {'attrib': 'pscale', 'singlevalue': 2}),
    ('i_new_group', 'attribadjustinteger', {'group': 'grp', 'attrib': 'newi', 'operation': 'set', 'singlevalue': 6}),
    # ---------------- color ----------------
    ('c_const', 'attribadjustcolor', {'singlevaluer': 0.2, 'singlevalueg': 0.4, 'singlevalueb': 0.6}),
    ('c_new_group', 'attribadjustcolor', {'group': 'grp', 'attrib': 'Cnew', 'singlevaluer': 0.25, 'singlevalueg': 0.5,
                                          'singlevalueb': 0.0}),
    ('c_add', 'attribadjustcolor', {'operation': 'add', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_sub', 'attribadjustcolor', {'operation': 'sub', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_mult', 'attribadjustcolor', {'operation': 'mult', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_overlay', 'attribadjustcolor', {'operation': 'overlay', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_screen', 'attribadjustcolor', {'operation': 'screen', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_hardmix', 'attribadjustcolor', {'operation': 'hardmix', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_diff', 'attribadjustcolor', {'operation': 'diff', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_min', 'attribadjustcolor', {'operation': 'min', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_max', 'attribadjustcolor', {'operation': 'max', 'singlevaluer': 0.3, 'singlevalueg': 0.6, 'singlevalueb': 0.9}),
    ('c_init', 'attribadjustcolor', {'attrib': 'Cnew', 'operation': 'init', 'singlevaluer': 0.3}),
    ('c_rand_rgb', 'attribadjustcolor', {'valuetype': 'rand', 'rangemethod': 'minmax', 'randrmin': 0.2, 'randgmax': 0.5}),
    ('c_rand_rgb_base', 'attribadjustcolor', {'valuetype': 'rand', 'rangemethod': 'minmax', 'dobasecolor': 1,
                                              'basecolorr': 0.5, 'basecolorg': 0.2, 'basecolorb': 0.1, 'randrmin': -0.2,
                                              'randrmax': 0.2, 'randgmin': 0.0, 'randgmax': 0.3, 'randbmin': -0.1,
                                              'randbmax': 0.1, 'randomseed': 5}),
    ('c_rand_hsv', 'attribadjustcolor', {'valuetype': 'rand', 'rangemethod': 'minmax', 'randomcolormodel': 'hsv',
                                         'randsatmin': 0.4, 'randvalmin': 0.5}),
    ('c_rand_hsv_base', 'attribadjustcolor', {'valuetype': 'rand', 'rangemethod': 'minmax', 'randomcolormodel': 'hsv',
                                              'dobasecolor': 1, 'basecolorr': 0.8, 'basecolorg': 0.3, 'basecolorb': 0.2,
                                              'randhuemin': -30.0, 'randhuemax': 30.0, 'randsatmin': -0.2,
                                              'randsatmax': 0.1, 'randvalmin': -0.3, 'randvalmax': 0.0}),
    ('c_rand_ramp', 'attribadjustcolor', {'valuetype': 'rand'}),
    ('c_line_ramp', 'attribadjustcolor', {'valuetype': 'line', 'startposx': -1.0, 'endposx': 1.5, 'endposz': 1.0}),
    ('c_bbox_ramp_prim', 'attribadjustcolor', {'class': 'primitive', 'attrib': 'Cd', 'valuetype': 'rand', 'randomseed': 9}),
    ('c_attrib', 'attribadjustcolor', {'attrib': 'Cnew', 'valuetype': 'attrib', 'adjustmentattrib': 'Cd',
                                       'adjustmentattribgain': 2.0, 'adjustmentattribbrightness': -0.1}),
    ('c_cc_hsv', 'attribadjustcolor', {'adjustvalue': 0, 'enable_cc': 1, 'dohueshift': 1, 'hueshift': 30.0, 'dosat': 1,
                                       'sat': 0.5, 'doval': 1, 'val': 1.2}),
    ('c_cc_rgb', 'attribadjustcolor', {'adjustvalue': 0, 'enable_cc': 1, 'dobrightness': 1, 'brightness': 0.1,
                                       'docontrast': 1, 'contrast': 1.3, 'dogain': 1, 'gain': 0.8, 'dogamma': 1,
                                       'gamma': 2.2}),
    ('c_noop_roundtrip', 'attribadjustcolor', {'adjustvalue': 0}),
    ('c_clamp_complement', 'attribadjustcolor', {'operation': 'add', 'singlevaluer': 0.3, 'singlevalueg': -0.2,
                                                 'singlevalueb': 0.0, 'enable_postprocess': 1, 'doclampmin': 1,
                                                 'clampminvalueg': 0.1, 'doclampmax': 1, 'clampmaxvaluer': 0.9,
                                                 'docomplement': 1}),
    ('c_preprocess_init', 'attribadjustcolor', {'enable_preprocess': 1, 'doinitvalue': 1, 'initvaluer': 0.5,
                                                'initvalueg': 0.25, 'initvalueb': 0.1, 'operation': 'add',
                                                'singlevaluer': 0.1, 'singlevalueg': 0.1, 'singlevalueb': 0.1}),
    ('c_preprocess_float', 'attribadjustcolor', {'enable_preprocess': 1, 'doinitvalue': 1, 'initvaluefrom': 'floatattrib',
                                                 'initfloatattrib': 'mask', 'adjustvalue': 0}),
    ('c_wrong_type', 'attribadjustcolor', {'attrib': 'pscale'}),
]

CLASS = {'point': hou.attribType.Point, 'vertex': hou.attribType.Vertex, 'primitive': hou.attribType.Prim,
         'detail': hou.attribType.Global}


def elems(g, cls):
    if cls == 'point':
        return g.points()
    if cls == 'primitive':
        return g.prims()
    if cls == 'vertex':
        return [v for pr in g.prims() for v in pr.vertices()]
    return None


records = []
base = build.geometry()
for name, typ, parms in cases:
    n = geo.createNode(typ, name)
    n.setInput(0, build)
    for k, v in parms.items():
        p = n.parm(k)
        if p is None:
            raise RuntimeError('%s: no parm %s' % (name, k))
        p.set(v)
    # OnCreated picks per-session seeds: record them so the case is complete
    parms = dict(parms)
    for k in ('randomseed', 'outlierseed'):
        if n.parm(k) is not None and k not in parms:
            parms[k] = n.parm(k).eval()
    g = n.geometry()
    cls = n.parm('class').evalAsString()
    attrib = n.parm('attrib').evalAsString()
    rec = {'name': name, 'type': typ, 'parms': parms, 'class': cls, 'attrib': attrib,
           'warnings': list(n.warnings()), 'errors': list(n.errors())}
    a = g.findGlobalAttrib(attrib) if cls == 'detail' else {
        'point': g.findPointAttrib, 'vertex': g.findVertexAttrib, 'primitive': g.findPrimAttrib}[cls](attrib)
    b = base.findGlobalAttrib(attrib) if cls == 'detail' else {
        'point': base.findPointAttrib, 'vertex': base.findVertexAttrib, 'primitive': base.findPrimAttrib}[cls](attrib)
    if a is None:
        rec['missing'] = True
    else:
        rec['data_type'] = a.dataType().name()
        rec['size'] = a.size()
        rec['type_info'] = a.qualifier()
        rec['default'] = list(a.defaultValue()) if isinstance(a.defaultValue(), tuple) else [a.defaultValue()]
        if cls == 'detail':
            vals = [g.attribValue(attrib)]
            old = [base.attribValue(attrib)] if b is not None else None
        else:
            vals = [e.attribValue(a) for e in elems(g, cls)]
            old = [e.attribValue(b) for e in elems(base, cls)] if b is not None else None
        rec['values'] = vals
        rec['unchanged'] = old == vals and b is not None and b.dataType() == a.dataType()
    records.append(rec)
    print(name, rec.get('data_type'), rec.get('unchanged'), rec['warnings'], rec['errors'])

with open(os.path.join(out, 'cases.json'), 'w', encoding='utf-8', newline='\n') as f:
    f.write('{"houdini": %s, "cases": [\n' % json.dumps(hou.applicationVersionString()))
    f.write(',\n'.join(json.dumps(r, sort_keys=True, separators=(',', ':')) for r in records))
    f.write('\n]}\n')
