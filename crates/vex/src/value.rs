//! Runtime values + the six VEX-subset types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Vec2,
    Vec3,
    Vec4,
    Str,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::Int => "int",
            Ty::Float => "float",
            Ty::Vec2 => "vector2",
            Ty::Vec3 => "vector",
            Ty::Vec4 => "vector4",
            Ty::Str => "string",
        }
    }
    /// Declaration keyword → type.
    pub fn from_keyword(s: &str) -> Option<Ty> {
        Some(match s {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "vector2" => Ty::Vec2,
            "vector" => Ty::Vec3,
            "vector4" => Ty::Vec4,
            "string" => Ty::Str,
            _ => return None,
        })
    }
    /// `i@x` style prefix → type.
    pub fn from_prefix(s: &str) -> Option<Ty> {
        Some(match s {
            "i" => Ty::Int,
            "f" => Ty::Float,
            "v" => Ty::Vec3,
            "u" | "v2" => Ty::Vec2,
            "p" | "v4" => Ty::Vec4,
            "s" => Ty::Str,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    Vec4([f64; 4]),
    Str(String),
}

impl Value {
    pub fn ty(&self) -> Ty {
        match self {
            Value::Int(_) => Ty::Int,
            Value::Float(_) => Ty::Float,
            Value::Vec2(_) => Ty::Vec2,
            Value::Vec3(_) => Ty::Vec3,
            Value::Vec4(_) => Ty::Vec4,
            Value::Str(_) => Ty::Str,
        }
    }
    pub fn zero(t: Ty) -> Value {
        match t {
            Ty::Int => Value::Int(0),
            Ty::Float => Value::Float(0.0),
            Ty::Vec2 => Value::Vec2([0.0; 2]),
            Ty::Vec3 => Value::Vec3([0.0; 3]),
            Ty::Vec4 => Value::Vec4([0.0; 4]),
            Ty::Str => Value::Str(String::new()),
        }
    }
    /// Scalar → f64 (`None` for vectors/strings).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }
    /// Components of a vector (`None` for scalars/strings).
    pub fn comps(&self) -> Option<&[f64]> {
        match self {
            Value::Vec2(v) => Some(v),
            Value::Vec3(v) => Some(v),
            Value::Vec4(v) => Some(v),
            _ => None,
        }
    }
    /// Build a vector of 2..=4 components.
    pub fn from_comps(c: &[f64]) -> Option<Value> {
        match *c {
            [a, b] => Some(Value::Vec2([a, b])),
            [a, b, c] => Some(Value::Vec3([a, b, c])),
            [a, b, c, d] => Some(Value::Vec4([a, b, c, d])),
            _ => None,
        }
    }
    /// Assignment conversion: int↔float (float→int truncates, saturating); same-shape otherwise; anything else `None`.
    pub fn coerce(&self, t: Ty) -> Option<Value> {
        match (self, t) {
            (Value::Int(i), Ty::Int) => Some(Value::Int(*i)),
            (Value::Int(i), Ty::Float) => Some(Value::Float(*i as f64)),
            (Value::Float(f), Ty::Float) => Some(Value::Float(*f)),
            (Value::Float(f), Ty::Int) => Some(Value::Int(*f as i64)),
            (v, t) if v.ty() == t => Some(v.clone()),
            _ => None,
        }
    }
    /// Condition truthiness: non-zero scalar, any non-zero vector component, non-empty string.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            v => v.comps().is_some_and(|c| c.iter().any(|x| *x != 0.0)),
        }
    }
}

/// Built-in attribute types (bare `@P` etc.); `None` → caller infers (existing attribute, else float).
pub fn builtin_ty(name: &str) -> Option<Ty> {
    Some(match name {
        "P" | "N" | "Cd" | "v" => Ty::Vec3,
        "ptnum" | "primnum" | "vtxnum" | "numpt" | "numprim" => Ty::Int,
        "Frame" | "Time" => Ty::Float,
        _ => return None,
    })
}

/// Built-ins the snippet may read but never write.
pub fn is_readonly(name: &str) -> bool {
    matches!(name, "ptnum" | "primnum" | "vtxnum" | "numpt" | "numprim" | "Frame" | "Time")
}
