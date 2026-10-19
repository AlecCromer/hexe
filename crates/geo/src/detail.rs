use crate::types::*;

impl AttrData {
    /// Floats per element.
    pub fn tuple_size(&self) -> u32 {
        match self {
            AttrData::F32(_) | AttrData::I32(_) | AttrData::Str { .. } => 1,
            AttrData::V2(_) => 2,
            AttrData::V3(_) => 3,
            AttrData::V4(_) => 4,
            AttrData::FN { size, .. } | AttrData::IN { size, .. } => *size,
        }
    }
    /// Element count.
    pub fn len(&self) -> usize {
        match self {
            AttrData::F32(v) => v.len(),
            AttrData::I32(v) => v.len(),
            AttrData::V2(v) => v.len(),
            AttrData::V3(v) => v.len(),
            AttrData::V4(v) => v.len(),
            AttrData::Str { idx, .. } => idx.len(),
            AttrData::FN { size, data } => data.len().checked_div(*size as usize).unwrap_or(0),
            AttrData::IN { size, data } => data.len().checked_div(*size as usize).unwrap_or(0),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Same variant + same tuple size (string tables compare equal-or-merge handled in `extend_from`).
    fn same_shape(&self, o: &AttrData) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(o) && self.tuple_size() == o.tuple_size()
    }
    /// Append `n` default elements (0 / -1 for strings).
    pub fn extend_default(&mut self, n: usize) {
        match self {
            AttrData::F32(v) => v.resize(v.len().saturating_add(n), 0.0),
            AttrData::I32(v) => v.resize(v.len().saturating_add(n), 0),
            AttrData::V2(v) => v.resize(v.len().saturating_add(n), [0.0; 2]),
            AttrData::V3(v) => v.resize(v.len().saturating_add(n), [0.0; 3]),
            AttrData::V4(v) => v.resize(v.len().saturating_add(n), [0.0; 4]),
            AttrData::Str { idx, .. } => idx.resize(idx.len().saturating_add(n), -1),
            AttrData::FN { size, data } => {
                let add = n.saturating_mul(*size as usize);
                data.resize(data.len().saturating_add(add), 0.0)
            }
            AttrData::IN { size, data } => {
                let add = n.saturating_mul(*size as usize);
                data.resize(data.len().saturating_add(add), 0)
            }
        }
    }
    /// Prepend `n` default elements.
    fn prepend_default(&mut self, n: usize) {
        let mut head = self.clone();
        head.clear();
        head.extend_default(n);
        head.extend_from(self);
        *self = head;
    }
    fn clear(&mut self) {
        match self {
            AttrData::F32(v) => v.clear(),
            AttrData::I32(v) => v.clear(),
            AttrData::V2(v) => v.clear(),
            AttrData::V3(v) => v.clear(),
            AttrData::V4(v) => v.clear(),
            AttrData::Str { idx, .. } => idx.clear(),
            AttrData::FN { data, .. } => data.clear(),
            AttrData::IN { data, .. } => data.clear(),
        }
    }
    /// Append `o`'s elements. Same shape required; mismatch → defaults appended instead (returns false).
    pub fn extend_from(&mut self, o: &AttrData) -> bool {
        if !self.same_shape(o) {
            self.extend_default(o.len());
            return false;
        }
        match (self, o) {
            (AttrData::F32(a), AttrData::F32(b)) => a.extend_from_slice(b),
            (AttrData::I32(a), AttrData::I32(b)) => a.extend_from_slice(b),
            (AttrData::V2(a), AttrData::V2(b)) => a.extend_from_slice(b),
            (AttrData::V3(a), AttrData::V3(b)) => a.extend_from_slice(b),
            (AttrData::V4(a), AttrData::V4(b)) => a.extend_from_slice(b),
            (AttrData::FN { data: a, .. }, AttrData::FN { data: b, .. }) => a.extend_from_slice(b),
            (AttrData::IN { data: a, .. }, AttrData::IN { data: b, .. }) => a.extend_from_slice(b),
            (AttrData::Str { idx: ai, table: at }, AttrData::Str { idx: bi, table: bt }) => {
                // remap b's table into a's
                let mut map = Vec::with_capacity(bt.len());
                for s in bt {
                    let pos = match at.iter().position(|x| x == s) {
                        Some(p) => p,
                        None => {
                            at.push(s.clone());
                            at.len() - 1
                        }
                    };
                    map.push(i32::try_from(pos).unwrap_or(-1));
                }
                for &i in bi {
                    let m = usize::try_from(i).ok().and_then(|u| map.get(u).copied()).unwrap_or(-1);
                    ai.push(m);
                }
            }
            _ => return false,
        }
        true
    }
}

impl Detail {
    pub fn vertex_count(&self) -> u32 {
        u32::try_from(self.vertex_point.len()).unwrap_or(u32::MAX)
    }
    pub fn prim_count(&self) -> u32 {
        u32::try_from(self.prims.len()).unwrap_or(u32::MAX)
    }
    /// Element count of an attribute class (Detail = 1).
    pub fn class_len(&self, c: AttrClass) -> usize {
        match c {
            AttrClass::Point => self.point_count as usize,
            AttrClass::Vertex => self.vertex_point.len(),
            AttrClass::Prim => self.prims.len(),
            AttrClass::Detail => 1,
        }
    }

    /// Point positions; empty if `P` is absent (never panics).
    pub fn p(&self) -> &[[f32; 3]] {
        match self.attr(AttrClass::Point, "P").map(|a| &a.data) {
            Some(AttrData::V3(v)) => v,
            _ => &[],
        }
    }
    /// Mutable positions; creates a zeroed `P` (V3, Point) if missing or wrongly typed.
    pub fn p_mut(&mut self) -> &mut Vec<[f32; 3]> {
        let n = self.point_count as usize;
        let ok = matches!(self.attr(AttrClass::Point, "P").map(|a| &a.data), Some(AttrData::V3(_)));
        if !ok {
            self.attrs.retain(|a| !(a.class == AttrClass::Point && a.name == "P"));
            self.attrs.insert(
                0,
                Attribute {
                    name: "P".into(),
                    class: AttrClass::Point,
                    data: AttrData::V3(vec![[0.0; 3]; n]),
                    type_info: TypeInfo::Point,
                },
            );
        }
        let idx = self.attrs.iter().position(|a| a.class == AttrClass::Point && a.name == "P").unwrap_or(0);
        match self.attrs.get_mut(idx).map(|a| &mut a.data) {
            Some(AttrData::V3(v)) => v,
            // Unreachable by construction (P was just ensured); stays total instead of panicking.
            _ => Box::leak(Box::default()),
        }
    }
    pub fn attr(&self, class: AttrClass, name: &str) -> Option<&Attribute> {
        self.attrs.iter().find(|a| a.class == class && a.name == name)
    }
    pub fn attr_mut(&mut self, class: AttrClass, name: &str) -> Option<&mut Attribute> {
        self.attrs.iter_mut().find(|a| a.class == class && a.name == name)
    }
    /// Add (replacing a same-name/class attribute). Data is padded/truncated to the class length.
    pub fn add_attr(&mut self, mut a: Attribute) -> &mut Attribute {
        let n = self.class_len(a.class);
        if a.data.len() < n {
            let missing = n - a.data.len();
            a.data.extend_default(missing);
        }
        self.attrs.retain(|x| !(x.class == a.class && x.name == a.name));
        self.attrs.push(a);
        let last = self.attrs.len() - 1;
        &mut self.attrs[last]
    }
    pub fn group(&self, class: AttrClass, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.class == class && g.name == name)
    }
    pub fn edge_group(&self, name: &str) -> Option<&EdgeGroup> {
        self.edge_groups.iter().find(|g| g.name == name)
    }
    /// Create / replace edge group `name`: each unordered pair kept once (first orientation wins), self-edges and pairs
    /// naming points past `point_count` dropped.
    pub fn set_edge_group(&mut self, name: &str, edges: impl IntoIterator<Item = [PointId; 2]>) {
        let n = self.point_count;
        let mut seen = std::collections::HashSet::new();
        let edges: Vec<[PointId; 2]> =
            edges.into_iter().filter(|&[a, b]| a != b && a < n && b < n && seen.insert((a.min(b), a.max(b)))).collect();
        match self.edge_groups.iter_mut().find(|g| g.name == name) {
            Some(g) => g.edges = edges,
            None => self.edge_groups.push(EdgeGroup { name: name.to_string(), edges }),
        }
    }
    /// Append `n` points (all point attrs and point groups extended with defaults / false). Returns the new id range.
    pub fn add_points(&mut self, n: u32) -> std::ops::Range<PointId> {
        let start = self.point_count;
        let end = start.saturating_add(n);
        let add = (end - start) as usize;
        self.point_count = end;
        for a in self.attrs.iter_mut().filter(|a| a.class == AttrClass::Point) {
            a.data.extend_default(add);
        }
        for g in self.groups.iter_mut().filter(|g| g.class == AttrClass::Point) {
            g.mask.resize(end as usize, false);
        }
        if self.attr(AttrClass::Point, "P").is_none() {
            let _ = self.p_mut();
        }
        start..end
    }
    /// Add a polygon over existing points. Out-of-range point ids are clamped away (dropped) — never panics.
    pub fn add_poly(&mut self, pts: &[PointId], closed: bool) -> PrimId {
        let start = self.vertex_count();
        for &p in pts.iter().filter(|&&p| p < self.point_count) {
            self.vertex_point.push(p);
        }
        let end = self.vertex_count();
        let id = self.prim_count();
        self.prims.push(Prim { kind: PrimKind::Poly { closed }, verts: start..end });
        for a in self.attrs.iter_mut().filter(|a| a.class == AttrClass::Vertex) {
            a.data.extend_default((end - start) as usize);
        }
        for a in self.attrs.iter_mut().filter(|a| a.class == AttrClass::Prim) {
            a.data.extend_default(1);
        }
        for g in self.groups.iter_mut() {
            match g.class {
                AttrClass::Vertex => g.mask.resize(end as usize, false),
                AttrClass::Prim => g.mask.resize(id as usize + 1, false),
                _ => {}
            }
        }
        id
    }
    /// Append `other` (points, vertices, prims, attributes, groups). Attributes/groups present on one side are default-filled.
    pub fn merge(&mut self, other: &Detail) {
        let (np, nv, nq) = (self.point_count, self.vertex_count(), self.prim_count());
        let (op, ov, oq) = (other.point_count as usize, other.vertex_point.len(), other.prims.len());
        for c in [AttrClass::Point, AttrClass::Vertex, AttrClass::Prim] {
            let (n_self, n_other) = match c {
                AttrClass::Point => (np as usize, op),
                AttrClass::Vertex => (nv as usize, ov),
                _ => (nq as usize, oq),
            };
            for a in self.attrs.iter_mut().filter(|a| a.class == c) {
                match other.attr(c, &a.name) {
                    Some(b) => {
                        a.data.extend_from(&b.data);
                    }
                    None => a.data.extend_default(n_other),
                }
            }
            for b in other.attrs.iter().filter(|b| b.class == c) {
                if self.attr(c, &b.name).is_none() {
                    let mut d = b.data.clone();
                    d.prepend_default(n_self);
                    self.attrs.push(Attribute { name: b.name.clone(), class: c, data: d, type_info: b.type_info });
                }
            }
            for g in self.groups.iter_mut().filter(|g| g.class == c) {
                let src = other.group(c, &g.name).map(|x| x.mask.as_slice()).unwrap_or(&[]);
                g.mask.resize(n_self, false);
                g.mask.extend((0..n_other).map(|i| src.get(i).copied().unwrap_or(false)));
            }
            let new_groups: Vec<Group> = other
                .groups
                .iter()
                .filter(|g| g.class == c && self.group(c, &g.name).is_none())
                .map(|g| {
                    let mut m = vec![false; n_self];
                    m.extend_from_slice(&g.mask);
                    m.resize(n_self + n_other, false);
                    Group { name: g.name.clone(), class: c, mask: m }
                })
                .collect();
            self.groups.extend(new_groups);
        }
        for b in other.attrs.iter().filter(|b| b.class == AttrClass::Detail) {
            if self.attr(AttrClass::Detail, &b.name).is_none() {
                self.attrs.push(b.clone());
            }
        }
        // edge groups: same-named ones join, `other`'s edges shifted by its new point offset
        for g in &other.edge_groups {
            let shifted = g.edges.iter().map(|&[a, b]| [a.saturating_add(np), b.saturating_add(np)]);
            match self.edge_groups.iter_mut().find(|e| e.name == g.name) {
                Some(e) => e.edges.extend(shifted),
                None => self.edge_groups.push(EdgeGroup { name: g.name.clone(), edges: shifted.collect() }),
            }
        }
        self.point_count = np.saturating_add(other.point_count);
        let shift_v = nv;
        self.vertex_point.extend(other.vertex_point.iter().map(|p| p.saturating_add(np)));
        self.prims.extend(other.prims.iter().map(|p| Prim {
            kind: p.kind.clone(),
            verts: p.verts.start.saturating_add(shift_v)..p.verts.end.saturating_add(shift_v),
        }));
    }
    /// Canonical order (what the readers/writer produce): attrs by class Point,Vertex,Prim,Detail; groups by class Point,Vertex,Prim.
    /// Stable within a class.
    pub fn canonicalize(&mut self) {
        let rank = |c: AttrClass| match c {
            AttrClass::Point => 0,
            AttrClass::Vertex => 1,
            AttrClass::Prim => 2,
            AttrClass::Detail => 3,
        };
        self.attrs.sort_by_key(|a| rank(a.class));
        self.groups.sort_by_key(|g| rank(g.class));
    }
    /// Axis-aligned bounds of `P`; `([0;3],[0;3])` when empty.
    pub fn bbox(&self) -> ([f32; 3], [f32; 3]) {
        let p = self.p();
        let Some(first) = p.first() else { return ([0.0; 3], [0.0; 3]) };
        let (mut lo, mut hi) = (*first, *first);
        for q in p {
            for i in 0..3 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
        (lo, hi)
    }
}
