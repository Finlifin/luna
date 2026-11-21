//! End-to-end comptime pattern matching verification across sysroot, bytecode VM, and tree-walker.
use comptime_interpretor::{Budget, Interpreter, SourceResolvers, Value};
use elaboration::materialization::{DeclarationSource, SourceInvocationQueries};
use luna::frontend;
use luna::packages::{self, SourcePackage};
use nir::{Primitive, Term};
use resolve::DependencyEdge;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{seq}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn library_dir() -> PathBuf {
    if let Ok(path) = std::env::var("LUNA_TEST_SYSROOT") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

type IntrinsicBindingVec = Vec<(nir::intrinsics::Operation, String)>;

fn load_intrinsic_selectors() -> (IntrinsicBindingVec, IntrinsicBindingVec) {
    let conf = fs::read_to_string(library_dir().join("language-items.conf"))
        .expect("read language-items.conf");
    let mut comptime = Vec::new();
    let mut host = Vec::new();
    for line in conf.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().to_string();
        if let Some(name) = key.strip_prefix("comptime-intrinsic.") {
            if !value.starts_with("std.")
                && let Some(op) = nir::intrinsics::Operation::from_name(name)
            {
                comptime.push((op, value));
            }
        } else if let Some(name) = key.strip_prefix("host-intrinsic.")
            && !value.starts_with("std.")
            && let Some(op) = nir::intrinsics::Operation::from_name(name)
        {
            host.push((op, value));
        }
    }
    (comptime, host)
}

fn compile_with_sysroot(source: &str) -> Result<nir::program::Package, String> {
    compile_with_sysroot_mode(source, false)
}

fn compile_with_sysroot_mode(
    source: &str,
    comptime_comparison: bool,
) -> Result<nir::program::Package, String> {
    let dir = unique_temp_dir("luna-comptime-patterns");
    let _guard = TempDirGuard(dir.clone());
    let entry = dir.join("lib.fl");
    fs::write(&entry, source).unwrap();

    let mut all_packages = vec![SourcePackage {
        resolution_id: 0,
        identity: 100,
        name: "app".into(),
        entry,
    }];
    let mut all_deps = Vec::new();
    let lib = library_dir();
    let mut next_id = 1u32;
    let mut next_identity = 101u64;
    let mut builtin_id = None;
    let mut sysroot_ids = Vec::new();

    for name in ["builtin", "meta", "attrs"] {
        let sys_entry = lib.join(name).join("lib.fl");
        if !sys_entry.is_file() {
            continue;
        }
        let res_id = next_id;
        let ident = next_identity;
        next_id += 1;
        next_identity += 1;
        if name == "builtin" {
            builtin_id = Some(res_id);
        }
        sysroot_ids.push((name.to_string(), res_id, ident));
        all_packages.push(SourcePackage {
            resolution_id: res_id,
            identity: ident,
            name: name.to_string(),
            entry: sys_entry,
        });
    }

    for pkg in &all_packages {
        for (name, prov_id, _) in &sysroot_ids {
            if pkg.resolution_id != *prov_id {
                all_deps.push(DependencyEdge {
                    requester: pkg.resolution_id,
                    alias: name.clone(),
                    provider: *prov_id,
                });
            }
        }
    }

    let mut protocols = elaboration::program::Protocols::default();
    protocols
        .builtin_implementation_packages
        .extend(sysroot_ids.iter().map(|(_, _, ident)| *ident));

    let (ci, hi) = load_intrinsic_selectors();
    let selectors = luna::protocols::type_constructors::Selectors {
        comptime_intrinsics: &ci,
        host_intrinsics: &hi,
        copy: Some("builtin.protocols.resources.Copy"),
        drop: Some("builtin.protocols.resources.Drop"),
        splice: Some("builtin.protocols.staging.Splice"),
        meta_list: Some("meta.List"),
        tuple: Some("builtin.types.tuple.Tuple"),
        index: Some("builtin.protocols.ops.Index"),
        ordering: Some(if comptime_comparison {
            "meta.numeric.ordering.ComptimeOrd"
        } else {
            "builtin.numeric.comparison.Ord"
        }),
        equality: Some(if comptime_comparison {
            "meta.numeric.equality.ComptimeEq"
        } else {
            "builtin.numeric.comparison.Eq"
        }),
        iterator: Some("builtin.protocols.iterator.Iterator"),
        into_iterator: Some("builtin.protocols.iterator.IntoIterator"),
        list_pattern: Some("builtin.protocols.patterns.ListPatternView"),
        comptime_list_pattern: Some("meta.data.patterns.ComptimeListPatternView"),
        arithmetic: [
            Some("builtin.protocols.ops.Add"),
            Some("builtin.protocols.ops.Sub"),
            Some("builtin.protocols.ops.Mul"),
            Some("builtin.protocols.ops.Div"),
            Some("builtin.protocols.ops.Rem"),
            Some("builtin.protocols.ops.Neg"),
            Some("builtin.protocols.ops.Not"),
        ],
        ..Default::default()
    };

    let target = Some(nir::target::TargetData::new(64).unwrap());
    let (artifact, _) = packages::compile_with_target_and_inputs(
        &all_packages,
        &all_deps,
        0,
        Some(builtin_id.expect("builtin sysroot package must exist")),
        &protocols,
        selectors,
        target,
    )?;
    match artifact {
        frontend::Artifact::Nir(package) => Ok(package),
        _ => Err("expected NIR artifact".to_string()),
    }
}

fn fn_id(package: &nir::program::Package, name: &str) -> nir::DeclarationId {
    package
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("missing function `{name}`"))
        .declaration
}

fn assert_parity(package: &nir::program::Package, name: &str, args: Vec<Value>, expected: Value) {
    let source = DeclarationSource::Borrowed(package);
    let mut prep_steps = 2_000_000usize;
    let context = source.package_context(&mut prep_steps).unwrap();
    let queries = SourceInvocationQueries::in_source(source, &context, &[]);
    let types = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    let resolvers = || SourceResolvers {
        types: &types,
        methods: &methods,
        associated: &associated,
    };
    let mut vm_budget = Budget {
        steps: 2_000_000,
        ..Default::default()
    };
    let mut tree_budget = Budget {
        steps: 2_000_000,
        ..Default::default()
    };
    let id = fn_id(package, name);
    let mut vm = Interpreter::for_comptime_resolving_in_budget(
        package,
        &[],
        &[],
        &[],
        resolvers(),
        &mut vm_budget,
    )
    .unwrap();
    let mut tree = Interpreter::for_comptime_resolving_in_budget(
        package,
        &[],
        &[],
        &[],
        resolvers(),
        &mut tree_budget,
    )
    .unwrap()
    .with_reference_execution();
    let before = vm.bytecode_invocations();
    let vm_val = vm.invoke(id, args.clone()).unwrap();
    let tree_val = tree.invoke(id, args).unwrap();
    assert!(
        vm.bytecode_invocations() > before,
        "expected bytecode VM execution for `{name}`"
    );
    assert_eq!(vm_val, expected, "bytecode VM mismatch on `{name}`");
    assert_eq!(tree_val, expected, "tree-walker mismatch on `{name}`");
}

#[test]
fn type_patterns_nominal_heads_projections_and_concrete_non_matches() {
    let package = compile_with_sysroot(
        r#"
        struct Pair where A, B {
            first: A,
            second: B,
        }

        enum Choice where L, R {
            left(L),
            right(R),
        }

        newtype Wrap<T> = T;

        struct Fixed where T, N: usize {
            dummy: T,
        }

        struct Dependent where T, default_val: T {
            dummy: T,
        }

        mod inner_mod {
            pub struct Boxed where T {
                val: T,
            }
        }

        enum ErrKind { bad }

        comptime fn anon_rec(...props: T) -> T where T {
            props
        }

        comptime fn classify_nominal(ty: Type) -> Integer {
            ty match {
                Pair<B: builtin.u16, A: builtin.bool> => 1,
                Pair<A, B> if A == B => 2,
                Pair<_, _> => 3,
                Choice<L, _> if L == i32 => 4,
                Wrap<T> if T == u8 => 5,
                Fixed<T, 8> if T == u8 => 6,
                inner_mod.Boxed<T> if T == char => 7,
                Dependent<_, _> => 8,
                (X, Y) if X == bool and Y == i32 => 9,
                _ => 0,
            }
        }

        comptime fn run() -> Integer {
            if classify_nominal(Pair<bool, u16>) != 1 { return 1; }
            let pair_same = classify_nominal(Pair<i32, i32>);
            if pair_same != 2 { return 100 + pair_same; }
            if classify_nominal(Pair<i32, bool>) != 3 { return 3; }
            if classify_nominal(Choice<i32, bool>) != 4 { return 4; }
            if classify_nominal(Wrap<u8>) != 5 { return 5; }
            if classify_nominal(Fixed<u8, 8>) != 6 { return 6; }
            if classify_nominal(inner_mod.Boxed<char>) != 7 { return 7; }
            if classify_nominal(Dependent<bool, true>) != 8 { return 8; }
            if classify_nominal(Tuple<bool, i32>) != 9 { return 9; }
            -- Concrete closed non-tuple / non-nominal types must fall through cleanly rather than erroring:
            if classify_nominal(?i32) != 0 { return 10; }
            if classify_nominal(!ErrKind i32) != 0 { return 11; }
            if classify_nominal((anon_rec { x: 1 })'type) != 0 { return 12; }
            if classify_nominal(fn(i32) -> bool) != 0 { return 13; }
            if classify_nominal(meta.List<i32>) != 0 { return 14; }
            if classify_nominal(meta.Object) != 0 { return 15; }
            0
        }
        "#,
    )
    .unwrap();

    assert_parity(
        &package,
        "run",
        vec![],
        Value::Integer(comptime_runtime::Integer::from_i128(0)),
    );

    // Dynamic Type argument passed straight from host to both VMs:
    let ty = Term::Nominal {
        declaration: package
            .newtypes
            .iter()
            .find(|n| n.name == "Wrap")
            .unwrap()
            .declaration,
        arguments: vec![Term::Primitive(Primitive::U8)],
    };
    assert_parity(
        &package,
        "classify_nominal",
        vec![Value::Type(ty)],
        Value::Integer(comptime_runtime::Integer::from_i128(5)),
    );
}

#[test]
fn type_patterns_reject_quote_clauses_in_nominal_heads() {
    let err = compile_with_sysroot(
        r#"
        struct RuntimeDep where T, quote tag: lift T {
            dummy: T,
        }
        comptime fn inspect(ty: Type) -> bool {
            ty match {
                RuntimeDep<_, _> => true,
                _ => false,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        err.contains("runtime dependency clauses"),
        "unexpected error: {err}"
    );
}

#[test]
fn type_patterns_const_normal_forms_and_variadic_heads() {
    let package = compile_with_sysroot(
        r#"
        pure comptime fn MakeTag<K: Type, N: Integer> -> Type {
            K
        }
        const AliasTag = MakeTag;

        comptime fn inspect_type(ty: Type) -> Integer {
            ty match {
                meta.List<E> if E == bool => 10,
                meta.List<meta.List<Inner>> if Inner == u8 => 11,
                Tuple<Es: es> if es.length() == 3 => 12,
                Tuple<A, B> if A == u16 and B == char => 13,
                Tuple<> => 14,
                AliasTag<T, 7> if T == bool => 15,
                not (AliasTag<_, _> or meta.List<_> or Tuple<_, _>) => 16,
                _ => 0,
            }
        }

        comptime fn dependent_shape(ty: Type) -> bool {
            let pair_match = ty match {
                AliasTag<bool, x> => x == 42,
                _ => false,
            };
            pair_match
        }

        comptime fn run() -> bool {
            inspect_type(meta.List<bool>) == 10
                and inspect_type(meta.List<meta.List<u8>>) == 11
                and inspect_type(Tuple<u8, u16, u32>) == 12
                and inspect_type(Tuple<u16, char>) == 13
                and inspect_type(Tuple<>) == 14
                and inspect_type(MakeTag<bool, 7>) == 15
                and inspect_type(i32) == 16
                and dependent_shape(MakeTag<bool, 42>)
                and not dependent_shape(i32)
        }
        "#,
    )
    .unwrap();

    assert_parity(&package, "run", vec![], Value::Bool(true));
}

#[test]
fn list_patterns_named_rest_nesting_and_missing_rest_rejection() {
    let package = compile_with_sysroot(
        r#"
        comptime fn tail_of(xs: meta.List<T>) -> meta.List<T> where T {
            xs match {
                [_, ...rest] => rest,
                _ => [],
            }
        }

        comptime fn sum_list(xs: meta.List<Integer>) -> Integer {
            var cur = xs;
            var total: Integer = 0;
            while cur is [head, ...tail] do {
                total = total + head;
                cur = tail;
            }
            total
        }

        comptime fn nested_rest(matrix: meta.List<meta.List<Integer>>) -> Integer {
            matrix match {
                [[first, ...inner_tail], ...outer_tail] => {
                    first + inner_tail.length() * 10 + outer_tail.length() * 100
                },
                _ => -1,
            }
        }

        comptime fn edge_rests() -> bool {
            let empty_in: meta.List<Integer> = [];
            let empty_all = empty_in match {
                [...all] => all.length() == 0,
                _ => false,
            };
            let single: meta.List<Integer> = [99];
            let single_ok = single match {
                [only, ...empty] => only == 99 and empty.length() == 0,
                _ => false,
            };
            let two_elem: meta.List<Integer> = [3, 4, 5, 6];
            let two_ok = two_elem match {
                [a, b, ...rest] => a == 3 and b == 4 and rest.length() == 2 and rest[1] == 6,
                _ => false,
            };
            let poly_in: meta.List<bool> = [true, false, true];
            let poly_ok = tail_of(poly_in).length() == 2;
            empty_all and single_ok and two_ok and poly_ok
        }

        comptime fn run() -> bool {
            sum_list([1, 2, 3, 4, 5]) == 15
                and nested_rest([[7, 8, 9], [10], [11, 12]]) == 227
                and edge_rests()
        }
        "#,
    )
    .unwrap();

    assert_parity(&package, "run", vec![], Value::Bool(true));

    // A custom ComptimeListPatternView impl that inherits the unimplemented default
    // list_pattern_view_rest must accept wildcard `..._` and reject named `...tail`.
    let custom_wild_ok = compile_with_sysroot(
        r#"
        struct RepeatBool { count: usize, flag: bool }
        comptime impl meta.ComptimeListPatternView for RepeatBool {
            assoc Element: Type = bool;
            comptime fn list_pattern_view_get_len(self) -> usize { self.count }
            comptime fn list_pattern_view_get_element(self, _index: usize) -> bool { self.flag }
        }
        comptime fn has_at_least_one(r: RepeatBool) -> bool {
            r match {
                [first, ..._] => first,
                _ => false,
            }
        }
        comptime fn run() -> bool { has_at_least_one(RepeatBool { count: 2, flag: true }) }
        "#,
    )
    .unwrap();
    assert_parity(&custom_wild_ok, "run", vec![], Value::Bool(true));

    let custom_named_err = compile_with_sysroot(
        r#"
        struct RepeatBool { count: usize, flag: bool }
        comptime impl meta.ComptimeListPatternView for RepeatBool {
            assoc Element: Type = bool;
            comptime fn list_pattern_view_get_len(self) -> usize { self.count }
            comptime fn list_pattern_view_get_element(self, _index: usize) -> bool { self.flag }
        }
        comptime fn bad(r: RepeatBool) -> bool {
            r match {
                [_, ...tail] => tail.count > 0,
                _ => false,
            }
        }
        "#,
    )
    .unwrap_err();
    assert!(
        custom_named_err.contains("list rest patterns require a slicing protocol"),
        "unexpected error: {custom_named_err}"
    );
}

#[test]
fn half_float_ranges_negation_domains_and_custom_eq_ord() {
    let package = compile_with_sysroot(
        r#"
        newtype CodeId = i32;

        comptime fn zero_code_id() -> CodeId {
            let z: i32 = 0;
            z.as(CodeId)
        }

        mod code_consts {
            use app.{CodeId, zero_code_id};
            pub const ZERO: CodeId = zero_code_id();
        }

        impl builtin.Splice for CodeId {
            assoc RuntimeTarget: Type = CodeId;

            fn splice(comptime data: CodeId) -> CodeId {
                data.as(i32).as(CodeId)
            }
        }

        impl builtin.math.Eq for CodeId {
            fn eq(self, other: Self) -> bool {
                self.as(i32) == other.as(i32)
            }
        }

        struct BoxVal {
            tag: Type,
        }

        struct Meter {
            raw: i32,
        }

        impl builtin.Splice for Meter {
            assoc RuntimeTarget: Type = Meter;

            fn splice(comptime data: Meter) -> Meter {
                Meter { raw: data.raw }
            }
        }

        mod meter_consts {
            use app.Meter;
            pub const TARGET: Meter = Meter { raw: 10 };
        }

        impl builtin.math.Eq for Meter {
            fn eq(self, other: Self) -> bool {
                self.raw == other.raw
            }
        }

        impl builtin.math.Ord for Meter {
            fn cmp(self, other: Self) -> builtin.math.Order {
                if self.raw < other.raw {
                    builtin.math.Order.less
                } else if self.raw > other.raw {
                    builtin.math.Order.greater
                } else {
                    builtin.math.Order.equal
                }
            }
        }

        comptime fn classify_f16(x: f16) -> Integer {
            x match {
                not (-1000.0..=1000.0) => 99,
                0.5..2.0 => 1,
                2.0..=4.0 => 2,
                _ => 3,
            }
        }

        comptime fn classify_bf16(x: bf16) -> Integer {
            x match {
                ..0.0 => 1,
                0.0..=3.5 => 2,
                not (..=3.5) => 3,
            }
        }

        comptime fn anon_rec(...props: T) -> T where T {
            props
        }

        comptime fn test_negations(
            opt: ?f16,
            ty: Type,
            xs: meta.List<Integer>,
            bx: BoxVal,
            obj: meta.Object,
        ) -> bool {
            let o_ok = opt match {
                not null => true,
                _ => false,
            };
            let t_ok = ty match {
                not builtin.bool => true,
                _ => false,
            };
            let b_ok = bx match {
                not BoxVal { tag: builtin.bool } => true,
                _ => false,
            };
            let l_ok = xs match {
                not [] => true,
                _ => false,
            };
            let one_i32: i32 = 1;
            let rec = anon_rec { a: one_i32 };
            let rec_ok = rec match {
                not { a: 0 } => true,
                _ => false,
            };
            let obj_ok = obj match {
                not _ => false,
                _ => true,
            };
            o_ok and t_ok and b_ok and l_ok and rec_ok and obj_ok
        }

        fn test_meter_patterns(m: Meter) -> i32 {
            let lo = Meter { raw: 20 };
            let hi = Meter { raw: 30 };
            m match {
                meter_consts.TARGET => 100,
                lo..hi => 200,
                hi.. => 300,
                _ => 0,
            }
        }

        fn test_runtime_code_neg(cid: CodeId) -> bool {
            cid match {
                not code_consts.ZERO => true,
                _ => false,
            }
        }

        fn run_runtime() -> bool {
            let zero: i32 = 0;
            let seven: i32 = 7;
            test_meter_patterns(Meter { raw: 10 }) == 100
                and test_meter_patterns(Meter { raw: 25 }) == 200
                and test_meter_patterns(Meter { raw: 30 }) == 300
                and test_meter_patterns(Meter { raw: 5 }) == 0
                and test_runtime_code_neg(seven.as(CodeId))
                and not test_runtime_code_neg(zero.as(CodeId))
        }

        comptime fn run() -> bool {
            let z16: f16 = 0.0;
            let z_bf16: bf16 = 0.0;
            let nan16: f16 = z16 / z16;
            let nan_bf16: bf16 = z_bf16 / z_bf16;
            let f16_ok = classify_f16(1.0) == 1
                and classify_f16(2.0) == 2
                and classify_f16(nan16) == 99
                and classify_f16(-0.5) == 3;
            let bf16_ok = classify_bf16(-1.5) == 1
                and classify_bf16(3.5) == 2
                and classify_bf16(4.0) == 3
                and classify_bf16(nan_bf16) == 3;
            let obj: meta.Object = { a: 1 };
            let neg_ok = test_negations(
                1.5,
                i32,
                [1, 2],
                BoxVal { tag: i32 },
                obj,
            );
            let neg_reject_ok = not test_negations(
                null,
                bool,
                [],
                BoxVal { tag: bool },
                obj,
            );
            f16_ok and bf16_ok and neg_ok and neg_reject_ok
        }
        "#,
    )
    .unwrap();

    assert_parity(&package, "run", vec![], Value::Bool(true));
    assert_eq!(
        luna::execution::interpret(&package, "run_runtime", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn comptime_custom_eq_and_ord_patterns_parity() {
    let package = compile_with_sysroot_mode(
        r#"
        newtype CodeId = Integer;

        mod code_consts {
            use app.CodeId;
            pub const ZERO: CodeId = 0.as(CodeId);
        }

        comptime impl meta.ComptimeEq for CodeId {
            comptime fn eq(self, other: Self) -> bool {
                self.as(Integer) == other.as(Integer)
            }
        }

        struct Meter {
            raw: Integer,
        }

        mod meter_consts {
            use app.Meter;
            pub const TARGET: Meter = Meter { raw: 10 };
        }

        comptime impl meta.ComptimeEq for Meter {
            comptime fn eq(self, other: Self) -> bool {
                self.raw == other.raw
            }
        }

        comptime impl meta.ComptimeOrd for Meter {
            comptime fn cmp(self, other: Self) -> builtin.math.Order {
                if self.raw < other.raw {
                    builtin.math.Order.less
                } else if self.raw > other.raw {
                    builtin.math.Order.greater
                } else {
                    builtin.math.Order.equal
                }
            }
        }

        comptime fn classify_meter(m: Meter) -> Integer {
            let lo = Meter { raw: 20 };
            let hi = Meter { raw: 30 };
            m match {
                meter_consts.TARGET => 100,
                lo..hi => 200,
                hi.. => 300,
                _ => 0,
            }
        }

        comptime fn non_zero_code(c: CodeId) -> bool {
            c match {
                not code_consts.ZERO => true,
                _ => false,
            }
        }

        comptime fn run() -> bool {
            classify_meter(Meter { raw: 10 }) == 100
                and classify_meter(Meter { raw: 25 }) == 200
                and classify_meter(Meter { raw: 30 }) == 300
                and classify_meter(Meter { raw: 5 }) == 0
                and non_zero_code(7.as(CodeId))
                and not non_zero_code(0.as(CodeId))
        }
        "#,
        true,
    )
    .unwrap();

    assert_parity(&package, "run", vec![], Value::Bool(true));
}

#[test]
fn comptime_control_flow_value_ref_errors_optionals_and_rollback() {
    let package = compile_with_sysroot(
        r#"
        enum ErrA { missing(Integer) }
        enum ErrB { bad_flag(bool) }

        struct Acc {
            total: Integer,
        }

        impl Acc {
            comptime fn absorb(self, item: ?Integer) -> bool {
                if item is (v? if v > 0) do {
                    self match {
                        Acc { total: t } => {
                            self.total = t + v;
                        }
                    }
                    true
                } else {
                    false
                }
            }
        }

        comptime fn classify_error(res: ![ErrA, ErrB] Integer) -> Integer {
            res match {
                ok! => ok + 1,
                error ErrA.missing(n) => -n,
                error ErrB.bad_flag(f) => if f { -100 } else { -200 },
            }
        }

        comptime fn nested_optional(x: ??Integer) -> Integer {
            x match {
                (v?)? => v,
                null? => -1,
                null => -2,
            }
        }

        comptime fn or_rollback_and_control_flow(xs: meta.List<Integer>, pair: Tuple<Integer, Integer>) -> Integer {
            -- or-pattern must not leak partial binding side-effects if the first branch fails
            var trace: Integer = 0;
            let or_val = pair match {
                (a if do { trace = trace + 1; a > 10 }, _) or (_, a) => a + trace,
            };

            -- let-else destructuring
            let [first, ...rest] = xs else {
                return -999;
            };

            -- var destructuring + mutation
            var (lhs, rhs) = pair;
            lhs = lhs + first + rest.length();

            -- matches expression
            let is_positive = (lhs matches 1..);
            if is_positive {
                lhs + rhs + or_val
            } else {
                0
            }
        }

        comptime fn run() -> bool {
            var acc = Acc { total: 5 };
            let m1 = acc.absorb(10);
            let m2 = acc.absorb(null);
            let acc_ok = m1 and not m2 and acc.total == 15;

            let err_ok = classify_error(10) == 11
                and classify_error(error ErrA.missing(4)) == -4
                and classify_error(error ErrB.bad_flag(true)) == -100;

            let opt_ok = nested_optional(7) == 7
                and nested_optional(null) == -2;

            let flow_ok = or_rollback_and_control_flow([10, 20, 30], (3, 4)) == (3 + 10 + 2) + 4 + (4 + 1);
            acc_ok and err_ok and opt_ok and flow_ok
        }
        "#,
    )
    .unwrap();

    assert_parity(&package, "run", vec![], Value::Bool(true));
}
