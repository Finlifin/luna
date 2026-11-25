//! Integration tests for Milestone 5 of Wishlist 01:
//! - Complete `meta.ops` comptime operator twins (`ComptimeAdd..ComptimeNot`, `ComptimeIndex`)
//! - Comptime lookup priority (`Comptime*` in comptime contexts, runtime protocols in runtime)
//! - Primitive `comptime impl`s & `meta.List<T>` `ComptimeIndex<Integer>`
//! - `WL-01-28` (`Splice` polish): `BitVec<N>`, `!E T` auto-splice, generic `T :- Drop`,
//!   and `derive Splice` rejection of comptime-only types (`Type`, `Object`, `meta.List`, etc.)
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn custom_struct_comptime_arithmetic_and_unary_twins() {
    let source = r#"
        struct Vec2 {
            x: u64,
            y: u64,
        }

        comptime impl ComptimeAdd for Vec2 {
            assoc Output: Type = Vec2;
            pure comptime fn add(self, rhs: Vec2) -> Vec2 {
                let lx = self.x;
                let ly = self.y;
                let rx = rhs.x;
                let ry = rhs.y;
                if lx < 1000 {
                    if ly < 1000 {
                        if rx < 1000 {
                            if ry < 1000 {
                                return Vec2 { x: lx + rx, y: ly + ry };
                            }
                        }
                    }
                }
                Vec2 { x: 0, y: 0 }
            }
        }

        comptime impl ComptimeSub for Vec2 {
            assoc Output: Type = Vec2;
            pure comptime fn sub(self, rhs: Vec2) -> Vec2 {
                let lx = self.x;
                let ly = self.y;
                let rx = rhs.x;
                let ry = rhs.y;
                if lx < 1000 {
                    if ly < 1000 {
                        if rx <= lx {
                            if ry <= ly {
                                return Vec2 { x: lx - rx, y: ly - ry };
                            }
                        }
                    }
                }
                Vec2 { x: 0, y: 0 }
            }
        }

        comptime impl ComptimeMul<.Rhs = u64> for Vec2 {
            assoc Output: Type = Vec2;
            pure comptime fn mul(self, rhs: u64) -> Vec2 {
                let lx = self.x;
                let ly = self.y;
                if lx < 1000 {
                    if ly < 1000 {
                        if rhs < 1000 {
                            return Vec2 { x: lx * rhs, y: ly * rhs };
                        }
                    }
                }
                Vec2 { x: 0, y: 0 }
            }
        }

        comptime impl ComptimeDiv<.Rhs = u64> for Vec2 {
            assoc Output: Type = Vec2;
            pure comptime fn div(self, rhs: u64) -> Vec2 {
                let lx = self.x;
                let ly = self.y;
                if rhs > 0 {
                    Vec2 { x: lx / rhs, y: ly / rhs }
                } else {
                    Vec2 { x: 0, y: 0 }
                }
            }
        }

        comptime impl ComptimeRem<.Rhs = u64> for Vec2 {
            assoc Output: Type = Vec2;
            pure comptime fn rem(self, rhs: u64) -> Vec2 {
                let lx = self.x;
                let ly = self.y;
                if rhs > 0 {
                    Vec2 { x: lx % rhs, y: ly % rhs }
                } else {
                    Vec2 { x: 0, y: 0 }
                }
            }
        }

        struct SignedVal {
            v: i64,
        }

        comptime impl ComptimeNeg for SignedVal {
            assoc Output: Type = SignedVal;
            pure comptime fn neg(self) -> SignedVal {
                let lv = self.v;
                if lv > -1000 {
                    if lv < 1000 {
                        return SignedVal { v: -lv };
                    }
                }
                SignedVal { v: 0 }
            }
        }

        struct Flags {
            a: bool,
            b: bool,
        }

        comptime impl ComptimeNot for Flags {
            assoc Output: Type = Flags;
            pure comptime fn invert(self) -> Flags {
                Flags { a: not self.a, b: not self.b }
            }
        }

        comptime fn eval() -> bool {
            let sum = Vec2 { x: 10u64, y: 20u64 } + Vec2 { x: 4u64, y: 7u64 };
            let diff = Vec2 { x: 10u64, y: 20u64 } - Vec2 { x: 4u64, y: 7u64 };
            let scaled = Vec2 { x: 10u64, y: 20u64 } * 3u64;
            let halved = Vec2 { x: 30u64, y: 60u64 } / 2u64;
            let remmed = Vec2 { x: 30u64, y: 60u64 } % 7u64;
            let negated = -(SignedVal { v: 7 });
            let flipped = not (Flags { a: true, b: false });

            sum.x == 14u64 and sum.y == 27u64
                and diff.x == 6u64 and diff.y == 13u64
                and scaled.x == 30u64 and scaled.y == 60u64
                and halved.x == 15u64 and halved.y == 30u64
                and remmed.x == 2u64 and remmed.y == 4u64
                and negated.v == -7
                and not flipped.a and flipped.b
        }

        const CT_OK: bool = eval();

        fn run() -> bool {
            CT_OK
        }
    "#;
    assert_eq!(support::invoke(source, "eval").unwrap(), Value::Bool(true));
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn custom_struct_comptime_index_by_value() {
    let source = r#"
        enum Axis {
            x,
            y,
            z,
        }

        struct Triple {
            x: Integer,
            y: Integer,
            z: Integer,
        }

        comptime impl ComptimeIndex<Axis> for Triple {
            assoc Output: Type = Integer;
            comptime fn index(self, idx: Axis) -> Integer {
                idx match {
                    Axis.x => self.x,
                    Axis.y => self.y,
                    Axis.z => self.z,
                }
            }
        }

        comptime impl ComptimeIndex<u64> for Triple {
            assoc Output: Type = Integer;
            comptime fn index(self, idx: u64) -> Integer {
                if idx == 0u64 {
                    self.x
                } else if idx == 1u64 {
                    self.y
                } else {
                    self.z
                }
            }
        }

        comptime fn eval() -> bool {
            let t1 = Triple { x: 11, y: 22, z: 33 };
            let t2 = Triple { x: 11, y: 22, z: 33 };
            let t3 = Triple { x: 11, y: 22, z: 33 };
            (t1[Axis.x] + t2[Axis.z] + t3[1u64]) == 66
        }

        const CT_OK: bool = eval();

        fn run() -> bool {
            CT_OK
        }
    "#;
    assert_eq!(support::invoke(source, "eval").unwrap(), Value::Bool(true));
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn comptime_context_prefers_comptime_twins_over_runtime_operators() {
    let source = r#"
        struct Dual {
            v: u64,
        }

        -- Runtime protocols intentionally add +1000 / 2000 / 3000 offsets to distinguish dispatch phase.
        impl Add for Dual {
            assoc Output: Type = Dual;
            fn add(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv < 1000 {
                        return Dual { v: lv + rv + 1000 };
                    }
                }
                Dual { v: 0 }
            }
        }
        impl Sub for Dual {
            assoc Output: Type = Dual;
            fn sub(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv <= lv {
                        return Dual { v: lv - rv + 1000 };
                    }
                }
                Dual { v: 0 }
            }
        }
        impl Mul for Dual {
            assoc Output: Type = Dual;
            fn mul(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv < 1000 {
                        return Dual { v: lv * rv + 1000 };
                    }
                }
                Dual { v: 0 }
            }
        }
        impl Div for Dual {
            assoc Output: Type = Dual;
            fn div(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv > 0 {
                        if rv < 1000 {
                            return Dual { v: (lv / rv) + 1000 };
                        }
                    }
                }
                Dual { v: 0 }
            }
        }
        impl Rem for Dual {
            assoc Output: Type = Dual;
            fn rem(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv > 0 {
                        if rv < 1000 {
                            return Dual { v: (lv % rv) + 1000 };
                        }
                    }
                }
                Dual { v: 0 }
            }
        }
        impl Neg for Dual {
            assoc Output: Type = Dual;
            fn neg(self) -> Dual {
                let lv = self.v;
                if lv < 1000 {
                    Dual { v: 2000 - lv }
                } else {
                    Dual { v: 0 }
                }
            }
        }
        impl Not for Dual {
            assoc Output: Type = Dual;
            fn invert(self) -> Dual {
                let lv = self.v;
                if lv < 1000 {
                    Dual { v: 3000 - lv }
                } else {
                    Dual { v: 0 }
                }
            }
        }

        -- Comptime twins compute without the +1000/+2000/+3000 runtime offsets.
        comptime impl ComptimeAdd for Dual {
            assoc Output: Type = Dual;
            pure comptime fn add(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv < 1000 {
                        return Dual { v: lv + rv };
                    }
                }
                Dual { v: 0 }
            }
        }
        comptime impl ComptimeSub for Dual {
            assoc Output: Type = Dual;
            pure comptime fn sub(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv <= lv {
                        return Dual { v: lv - rv };
                    }
                }
                Dual { v: 0 }
            }
        }
        comptime impl ComptimeMul for Dual {
            assoc Output: Type = Dual;
            pure comptime fn mul(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv < 1000 {
                        return Dual { v: lv * rv };
                    }
                }
                Dual { v: 0 }
            }
        }
        comptime impl ComptimeDiv for Dual {
            assoc Output: Type = Dual;
            pure comptime fn div(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv > 0 {
                        if rv < 1000 {
                            return Dual { v: lv / rv };
                        }
                    }
                }
                Dual { v: 0 }
            }
        }
        comptime impl ComptimeRem for Dual {
            assoc Output: Type = Dual;
            pure comptime fn rem(self, rhs: Dual) -> Dual {
                let lv = self.v;
                let rv = rhs.v;
                if lv < 1000 {
                    if rv > 0 {
                        if rv < 1000 {
                            return Dual { v: lv % rv };
                        }
                    }
                }
                Dual { v: 0 }
            }
        }
        comptime impl ComptimeNeg for Dual {
            assoc Output: Type = Dual;
            pure comptime fn neg(self) -> Dual {
                let lv = self.v;
                if lv < 500 {
                    Dual { v: 500 - lv }
                } else {
                    Dual { v: 0 }
                }
            }
        }
        comptime impl ComptimeNot for Dual {
            assoc Output: Type = Dual;
            pure comptime fn invert(self) -> Dual {
                let lv = self.v;
                if lv < 500 {
                    Dual { v: 600 - lv }
                } else {
                    Dual { v: 0 }
                }
            }
        }

        comptime fn ct_check() -> bool {
            (Dual { v: 20u64 } + Dual { v: 6u64 }).v == 26u64
                and (Dual { v: 20u64 } - Dual { v: 6u64 }).v == 14u64
                and (Dual { v: 20u64 } * Dual { v: 6u64 }).v == 120u64
                and (Dual { v: 20u64 } / Dual { v: 6u64 }).v == 3u64
                and (Dual { v: 20u64 } % Dual { v: 6u64 }).v == 2u64
                and (-(Dual { v: 6u64 })).v == 494u64
                and (not (Dual { v: 6u64 })).v == 594u64
        }

        fn rt_check() -> bool {
            (Dual { v: 20u64 } + Dual { v: 6u64 }).v == 1026u64
                and (Dual { v: 20u64 } - Dual { v: 6u64 }).v == 1014u64
                and (Dual { v: 20u64 } * Dual { v: 6u64 }).v == 1120u64
                and (Dual { v: 20u64 } / Dual { v: 6u64 }).v == 1003u64
                and (Dual { v: 20u64 } % Dual { v: 6u64 }).v == 1002u64
                and (-(Dual { v: 6u64 })).v == 1994u64
                and (not (Dual { v: 6u64 })).v == 2994u64
        }

        const CT_OK: bool = ct_check();

        fn run() -> bool {
            CT_OK and rt_check()
        }
    "#;
    assert_eq!(
        support::invoke(source, "ct_check").unwrap(),
        Value::Bool(true)
    );
    let package = support::compile(source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 10_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_operators_reject_runtime_only_impls_and_comptime_pointers() {
    let runtime_add_in_comptime = r#"
        struct RtOnly { v: i64 }
        impl Add for RtOnly {
            assoc Output: Type = RtOnly;
            fn add(self, rhs: RtOnly) -> RtOnly { RtOnly { v: 1 } }
        }
        comptime fn bad() -> i64 {
            let a = RtOnly { v: 1 };
            let b = RtOnly { v: 2 };
            (a + b).v
        }
    "#;
    assert!(support::compile(runtime_add_in_comptime).is_err());

    let runtime_neg_in_comptime = r#"
        struct RtNeg { v: i64 }
        impl Neg for RtNeg {
            assoc Output: Type = RtNeg;
            fn neg(self) -> RtNeg { RtNeg { v: 0 } }
        }
        comptime fn bad() -> i64 {
            let a = RtNeg { v: 5 };
            (-a).v
        }
    "#;
    assert!(support::compile(runtime_neg_in_comptime).is_err());

    let runtime_not_in_comptime = r#"
        struct RtNot { b: bool }
        impl Not for RtNot {
            assoc Output: Type = RtNot;
            fn invert(self) -> RtNot { RtNot { b: not self.b } }
        }
        comptime fn bad() -> bool {
            let a = RtNot { b: true };
            (not a).b
        }
    "#;
    assert!(support::compile(runtime_not_in_comptime).is_err());

    let runtime_index_in_comptime = r#"
        struct Slot { p: *u64 }
        impl Index<u64> for Slot {
            assoc Output: Type = u64;
            fn index(*self, idx: u64) -> *u64 { self.*.p }
        }
        comptime fn bad(s: Slot) -> u64 {
            s[0u64]
        }
    "#;
    assert!(support::compile(runtime_index_in_comptime).is_err());

    let pointer_in_comptime = r#"
        comptime fn bad() -> u64 {
            var x: u64 = 1;
            let p = x.ref;
            p.*
        }
    "#;
    assert!(support::compile(pointer_in_comptime).is_err());
}

#[test]
fn primitive_comptime_impls_and_meta_list_comptime_index() {
    let source = r#"
        pure comptime fn generic_add(a: T, b: T) -> T.as(ComptimeAdd).Output
        where T :- ComptimeAdd {
            a + b
        }

        pure comptime fn generic_sub(a: T, b: T) -> T.as(ComptimeSub).Output
        where T :- ComptimeSub {
            a - b
        }

        pure comptime fn generic_mul(a: T, b: T) -> T.as(ComptimeMul).Output
        where T :- ComptimeMul {
            a * b
        }

        pure comptime fn generic_div(a: T, b: T) -> T.as(ComptimeDiv).Output
        where T :- ComptimeDiv {
            a / b
        }

        pure comptime fn generic_neg(x: T) -> T.as(ComptimeNeg).Output
        where T :- ComptimeNeg {
            -x
        }

        pure comptime fn generic_not(x: T) -> T.as(ComptimeNot).Output
        where T :- ComptimeNot {
            not x
        }

        comptime fn generic_index(c: C, idx: Integer) -> C.as(ComptimeIndex<Integer>).Output
        where C :- ComptimeIndex<Integer> {
            c[idx]
        }

        comptime fn eval() -> bool {
            let int_42: Integer = 42;
            let int_100: Integer = 100;
            let int_23: Integer = 23;
            let int_6: Integer = 6;
            let int_7: Integer = 7;
            let i_ok = generic_neg(int_42) == -42
                and generic_add(int_100, int_23) == 123
                and generic_sub(int_100, int_23) == 77
                and generic_mul(int_6, int_7) == 42;
            let flt_a: Float = 1.5;
            let flt_b: Float = 2.5;
            let flt_ok = generic_add(flt_a, flt_b) == 4.0
                and generic_neg(flt_b) == -2.5;
            let f1: f64 = 1.5;
            let f2: f64 = 2.5;
            let f3: f64 = 5.5;
            let f4: f64 = 2.0;
            let f5: f64 = 3.0;
            let f6: f64 = 7.5;
            let f_sum = generic_add(f1, f2);
            let f_diff = generic_sub(f3, f4);
            let f_prod = generic_mul(f5, f2);
            let f_quot = generic_div(f6, f2);
            let f_neg = generic_neg(f2);
            let s_lo: f64 = 3.99;
            let s_hi: f64 = 4.01;
            let d_lo: f64 = 3.49;
            let d_hi: f64 = 3.51;
            let p_lo: f64 = 7.49;
            let p_hi: f64 = 7.51;
            let q_lo: f64 = 2.99;
            let q_hi: f64 = 3.01;
            let n_lo: f64 = -2.51;
            let n_hi: f64 = -2.49;
            let f_ok = f_sum > s_lo and f_sum < s_hi
                and f_diff > d_lo and f_diff < d_hi
                and f_prod > p_lo and f_prod < p_hi
                and f_quot > q_lo and f_quot < q_hi
                and f_neg > n_lo and f_neg < n_hi;
            let bv_in = (0b0101u8'bitvec)[0..4];
            let bv_out = (0b1010u8'bitvec)[0..4];
            let not_ok = generic_not(true) == false
                and (not bv_in) == bv_out;
            let items: meta.List<i64> = [10i64, 20i64, 30i64];
            let idx_ok = generic_index(items, 1) == 20i64
                and items[2] == 30i64;
            i_ok and flt_ok and f_ok and not_ok and idx_ok
        }

        const ALL_OK: bool = eval();

        fn run() -> bool {
            ALL_OK
        }
    "#;
    assert_eq!(support::invoke(source, "eval").unwrap(), Value::Bool(true));
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn wl01_28_splice_polish_bitvec_error_result_drop_and_comptime_only_fields() {
    let ok_source = r#"
        enum ParseErr {
            Invalid(u8),
            Empty,
        }
        derive Splice for ParseErr;

        comptime fn ct_bits() -> BitVec<8> {
            0xA5u8'bitvec
        }

        comptime fn ct_ok() -> !ParseErr u16 {
            42u16
        }

        comptime fn ct_err() -> !ParseErr u16 {
            error ParseErr.Invalid(9u8)
        }

        const BITS: BitVec<8> = ct_bits();

        fn check_err(res: !ParseErr u16) -> u16 {
            res match {
                v! => v,
                error ParseErr.Invalid(code) => if code == 9u8 { 109u16 } else { 0u16 },
                error ParseErr.Empty => 0u16,
            }
        }

        fn run() -> bool {
            let b_direct = ct_bits() == 0xA5u8'bitvec;
            let b_const = BITS == 0xA5u8'bitvec;
            let ok_val = check_err(ct_ok());
            let err_val = check_err(ct_err());
            b_direct and b_const and ok_val == 42u16 and err_val == 109u16
        }
    "#;
    let package = support::compile(ok_source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 10_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );

    let generic_drop_splice = r#"
        comptime fn make_opt() -> ?T where T {
            null
        }
        fn use_drop() -> bool where T :- Splice + Drop {
            let _x: ?T = make_opt<T>();
            true
        }
    "#;
    let err = support::compile(generic_drop_splice).unwrap_err();
    assert!(
        err.contains("cannot auto-derive Splice for Drop type"),
        "unexpected error for generic T :- Drop auto-splice: {err}"
    );

    for bad_derive in [
        "struct HasType { t: Type } derive Splice for HasType;",
        "struct HasObj { o: Object } derive Splice for HasObj;",
        "struct HasList { xs: meta.List<u64> } derive Splice for HasList;",
        "struct HasInt { n: Integer } derive Splice for HasInt;",
        "struct HasFloat { f: Float } derive Splice for HasFloat;",
    ] {
        let err = support::compile(bad_derive).unwrap_err();
        assert!(
            err.contains("comptime-only"),
            "expected comptime-only rejection for `{bad_derive}`, got: {err}"
        );
    }
}
