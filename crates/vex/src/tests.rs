//! Interpreter tests against a map-backed mock host.
use crate::{ErrorKind, Host, HostError, Ty, Value, parse, run};
use std::collections::BTreeMap;

#[derive(Default)]
struct Mock {
    attrs: BTreeMap<String, Value>,
    added: Vec<[f64; 3]>,
    removed: Vec<i64>,
}

impl Host for Mock {
    fn attr(&mut self, name: &str, _h: Option<Ty>) -> Result<Option<Value>, HostError> {
        Ok(self.attrs.get(name).cloned())
    }
    fn set_attr(&mut self, name: &str, h: Option<Ty>, v: Value) -> Result<(), HostError> {
        let v = match (h, self.attrs.get(name)) {
            (_, Some(old)) => v.coerce(old.ty()),
            (Some(t), None) => v.coerce(t),
            (None, None) => v.coerce(Ty::Float),
        };
        match v {
            Some(v) => {
                self.attrs.insert(name.into(), v);
                Ok(())
            }
            None => Err(HostError::Failed("type mismatch".into())),
        }
    }
    fn add_point(&mut self, p: [f64; 3]) -> Result<i64, HostError> {
        self.added.push(p);
        Ok(8 + self.added.len() as i64)
    }
    fn remove_point(&mut self, n: i64) -> Result<(), HostError> {
        self.removed.push(n);
        Ok(())
    }
    fn add_prim(&mut self, _k: &str, p: &[i64]) -> Result<i64, HostError> {
        Ok(p.len() as i64)
    }
    fn set_point_attrib(&mut self, _n: &str, _p: i64, _v: Value) -> Result<(), HostError> {
        Err(HostError::Unsupported("mock".into()))
    }
    fn point(&mut self, _n: &str, _p: i64) -> Result<Value, HostError> {
        Ok(Value::Vec3([1.0, 2.0, 3.0]))
    }
    fn npoints(&mut self) -> i64 {
        9
    }
}

fn exec(src: &str, m: &mut Mock) -> Result<(), crate::VexError> {
    let p = parse(src)?;
    run(&p, m, &mut 1_000_000)
}
fn go(src: &str) -> Mock {
    let mut m = Mock::default();
    m.attrs.insert("P".into(), Value::Vec3([1.0, 2.0, 3.0]));
    exec(src, &mut m).unwrap_or_else(|e| panic!("{src}: {e}"));
    m
}
fn f(m: &Mock, k: &str) -> f64 {
    m.attrs.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

#[test]
fn component_read_write_and_compound() {
    let m = go("@P.y += 1; @P.z *= 2; @P.x -= 0.5;");
    assert_eq!(m.attrs["P"], Value::Vec3([0.5, 3.0, 6.0]));
}

#[test]
fn vector_arithmetic_elementwise_and_scalar() {
    let m = go("@P = @P * 2; v@a = {1,2,3} + {10,20,30}; v@b = 2 * {1,1,1} - 1;");
    assert_eq!(m.attrs["P"], Value::Vec3([2.0, 4.0, 6.0]));
    assert_eq!(m.attrs["a"], Value::Vec3([11.0, 22.0, 33.0]));
    assert_eq!(m.attrs["b"], Value::Vec3([1.0; 3]));
}

#[test]
fn attrs_created_with_prefix_type() {
    let m = go("@Cd = {1,0,0}; f@h = @P.x * 2 + 1; i@id = 7; s@name = 'hi'; @q = 3;");
    assert_eq!(m.attrs["Cd"], Value::Vec3([1.0, 0.0, 0.0]));
    assert_eq!(f(&m, "h"), 3.0);
    assert_eq!(m.attrs["id"], Value::Int(7));
    assert_eq!(m.attrs["name"], Value::Str("hi".into()));
    assert_eq!(m.attrs["q"], Value::Float(3.0));
}

#[test]
fn control_flow_loops_break_continue_return() {
    let m = go("for(int i=0;i<3;i++) @P.x += 1;");
    assert_eq!(m.attrs["P"], Value::Vec3([4.0, 2.0, 3.0]));
    let m = go("int s = 0; for(int i=0;i<10;i++){ if(i==3) continue; if(i==6) break; s += i; } f@s = s;");
    assert_eq!(f(&m, "s"), 0.0 + 1.0 + 2.0 + 4.0 + 5.0);
    let m = go("int n = 0; while(n < 5) n++; f@n = n; if(n == 5) { f@r = 1; return; } f@r = 2;");
    assert_eq!((f(&m, "n"), f(&m, "r")), (5.0, 1.0));
    let m = go("if(0) f@a = 1; else if(1) f@a = 2; else f@a = 3;");
    assert_eq!(f(&m, "a"), 2.0);
}

#[test]
fn builtins_oracles() {
    let m = go(
        "f@a = fit01(0.5, 10, 20); f@b = length({3,4,0}); f@c = dot({1,2,3},{4,5,6}); f@d = clamp(5, 0, 3); f@e = lerp(0, 10, 0.25); f@g = fit(5, 0, 10, 100, 200); f@h = pow(2, 10); f@i = atan2(1, 1); f@j = rint(2.5); f@k = max(1, 2.5); f@l = floor(-0.5); f@m = int(2.9); f@n = abs(-3); f@o = distance({1,1,1},{1,1,4});",
    );
    for (k, want) in [
        ("a", 15.0),
        ("b", 5.0),
        ("c", 32.0),
        ("d", 3.0),
        ("e", 2.5),
        ("g", 150.0),
        ("h", 1024.0),
        ("j", 2.0),
        ("k", 2.5),
        ("l", -1.0),
        ("m", 2.0),
        ("n", 3.0),
        ("o", 3.0),
    ] {
        assert_eq!(f(&m, k), want, "{k}");
    }
    assert!((f(&m, "i") - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
    let m = go("v@x = cross({1,0,0},{0,1,0}); v@n = normalize({0,3,4}); v@s = set(1,2,3);");
    assert_eq!(m.attrs["x"], Value::Vec3([0.0, 0.0, 1.0]));
    assert_eq!(m.attrs["n"], Value::Vec3([0.0, 0.6, 0.8]));
    assert_eq!(m.attrs["s"], Value::Vec3([1.0, 2.0, 3.0]));
}

#[test]
fn rand_and_noise_deterministic_in_range() {
    let m = go(
        "f@r0 = rand(0); f@r1 = rand(1); f@r1b = rand(1.0); f@rv = rand({1,2,3}); f@n = noise({0.3,0.7,1.1}); f@z = noise({1,2,3});",
    );
    assert_ne!(f(&m, "r0"), f(&m, "r1"));
    assert_eq!(f(&m, "r1"), f(&m, "r1b"));
    assert_eq!(f(&m, "z"), 0.5);
    for k in ["r0", "r1", "rv", "n"] {
        assert!((0.0..1.0).contains(&f(&m, k)) || k == "n", "{k}");
    }
    let m2 = go("f@r0 = rand(0);");
    assert_eq!(f(&m, "r0"), f(&m2, "r0"));
    for i in 0..500 {
        let m = go(&format!("f@r = rand({i}); f@n = noise(set({i}*0.37, 0.5, 1.7));"));
        assert!((0.0..1.0).contains(&f(&m, "r")) && (0.0..=1.0).contains(&f(&m, "n")));
    }
}

#[test]
fn division_by_zero_is_inf_not_panic() {
    let m = go("f@a = 1/0; f@b = -1/0.0; f@c = 0/0; i@d = 5 % 0; f@e = 7 / 2; f@g = 7 / 2.0;");
    assert_eq!(f(&m, "a"), f64::INFINITY);
    assert_eq!(f(&m, "b"), f64::NEG_INFINITY);
    assert!(f(&m, "c").is_nan());
    assert_eq!(m.attrs["d"], Value::Int(0));
    assert_eq!((f(&m, "e"), f(&m, "g")), (3.0, 3.5));
}

#[test]
fn long_loop_no_stack_blow_and_runaway_is_bounded() {
    let m = go("int s = 0; for(int i=0;i<10000;i++) s += i; i@s = s;");
    assert_eq!(m.attrs["s"], Value::Int(49_995_000));
    let m = go("int s = 0; int i = 0; while(i < 10000) { i++; if (i % 2) continue; s += 1; } i@s = s;");
    assert_eq!(m.attrs["s"], Value::Int(5000));
    for src in ["while(1) {}", "for(;;) {}", "for(;;) { }", "int i = 0; while(i >= 0) i += 0;"] {
        let mut m = Mock::default();
        let e = run(&parse(src).expect("parses"), &mut m, &mut 100_000).expect_err(src);
        assert_eq!(e.kind, ErrorKind::Runtime);
        assert!(e.msg.contains("budget"), "{src}");
    }
}

#[test]
fn type_mismatches_fail_with_position() {
    for (src, line, col) in [
        ("v@a = 1;", 1, 1),
        ("float x = {1,2,3};", 1, 11),
        ("vector v = {1,2,3};\n  float f = v + {1,2};", 2, 13),
        ("f@a = 'a' + 1;", 1, 7),
        ("x = 1;", 1, 1),
        ("f@a = nosuch(1);", 1, 7),
        ("i@ptnum = 3;", 1, 1),
        ("f@a = {1,'b',3}.x;", 1, 10),
        ("float f = 1; f.x = 2;", 1, 14),
        ("f@a = length(1,2);", 1, 7),
        ("f@a = addpoint(1, {0,0,0});", 1, 7),
    ] {
        let e = exec(src, &mut Mock::default()).expect_err(src);
        assert_eq!(e.kind, ErrorKind::Runtime, "{src}: {e}");
        assert_eq!((e.line, e.col), (line, col), "{src}: {e}");
    }
}

#[test]
fn geometry_builtins_route_to_host() {
    let m = go(
        "int n = addpoint(0, {5,5,5}); i@n = n; removepoint(0, 3); i@np = npoints(0); v@p = point(0, 'P', 1); printf('x %d', 1);",
    );
    assert_eq!(m.attrs["n"], Value::Int(9));
    assert_eq!((m.added.clone(), m.removed.clone()), (vec![[5.0; 3]], vec![3]));
    assert_eq!(m.attrs["np"], Value::Int(9));
    assert_eq!(m.attrs["p"], Value::Vec3([1.0, 2.0, 3.0]));
    let e = exec("setpointattrib(0, 'a', 0, 1.0);", &mut Mock::default()).expect_err("unsupported");
    assert_eq!(e.kind, ErrorKind::Unsupported);
}

#[test]
fn scoping_shadowing_and_ternary() {
    let m = go(
        "int a = 1; { int a = 5; f@in = a; } f@out = a; f@t = a > 0 ? 10 : 20; for(int a = 7; a < 8; a++) f@loop = a; f@after = a;",
    );
    assert_eq!((f(&m, "in"), f(&m, "out"), f(&m, "t"), f(&m, "loop"), f(&m, "after")), (5.0, 1.0, 10.0, 7.0, 1.0));
    let m = go("int i = 3; float x = i; x += 0.5; f@x = x; int j = 2.9; i@j = j;");
    assert_eq!((f(&m, "x"), m.attrs["j"].clone()), (3.5, Value::Int(2)));
}

#[test]
fn missing_attr_reads_zero_and_unprefixed_unknown_is_float() {
    let m = go("f@a = @nothing + 1; v@b = @Cd; i@c = @ptnum;");
    assert_eq!(f(&m, "a"), 1.0);
    assert_eq!(m.attrs["b"], Value::Vec3([0.0; 3]));
}

#[test]
fn hostile_snippets_never_panic() {
    for s in [
        "((((",
        "@",
        "}",
        "'",
        "1 +",
        "for(;;",
        "/*",
        "@P.w = 1;",
        "@P.x.y = 1;",
        "-'a';",
        "{1,2} * {1,2,3};",
        "{1,2,3} < {1,2,3};",
        "f@a = noise('a');",
    ] {
        let _ = exec(s, &mut go(""));
    }
}
