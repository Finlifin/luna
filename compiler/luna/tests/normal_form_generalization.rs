use luna::cli::{Command, Options, parse};
use luna::frontend::{Artifact, compile};
use nir::program::ExprKind as K;
use nir::{Primitive as P, Term};
use std::ffi::OsString;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn compile_nir(source: &str, bind_protocols: bool) -> Result<nir::program::Package, String> {
    let dir = std::env::temp_dir().join(format!(
        "luna-nf-general-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let path = dir.join("input.fl");
    let full = format!(
        "mod primitives {{\n{}\n}}\nuse primitives.*;\n{source}",
        include_str!("../../../library/builtin/primitives.fl"),
    );
    std::fs::write(&path, full).map_err(|err| err.to_string())?;

    let cmd = parse(vec![
        OsString::from("--emit"),
        OsString::from("nir"),
        path.into_os_string(),
    ])?;
    let Command::Compile(mut options): Command = cmd else {
        let _ = std::fs::remove_dir_all(&dir);
        return Err("expected compile command".into());
    };
    if bind_protocols {
        options.meta_list_type = Some("input.List".into());
        options.tuple_type = Some("input.Tuple".into());
    }
    let result = compile(&options);
    let _ = std::fs::remove_dir_all(&dir);
    match result? {
        Artifact::Nir(package) => Ok(package),
        other => Err(format!("unexpected artifact: {other}")),
    }
}

fn compile_options_only(options: &Options) -> Result<Artifact, String> {
    compile(options)
}

fn unwrap_tail<'a>(mut expr: &'a nir::program::Expr) -> &'a nir::program::Expr {
    while let K::Block {
        statements,
        result: Some(tail),
        ..
    } = &expr.kind
    {
        if statements.is_empty() {
            expr = tail;
        } else {
            break;
        }
    }
    expr
}

fn find_fn<'a>(package: &'a nir::program::Package, needle: &str) -> &'a nir::program::Function {
    package
        .functions
        .iter()
        .find(|f| f.name.contains(needle))
        .unwrap_or_else(|| panic!("missing function `{needle}`"))
}

#[test]
fn user_defined_variadic_normal_form_heads_support_positional_spread_and_named_lists() {
    let source = r#"
pub const List: pure comptime fn<T: Type> -> Type = .List'builtin;
pub const Tuple: pure comptime fn<...Es: List<Type>> -> Type = .Tuple'builtin;

pub pure comptime fn Tagged<tag: bool, ...Es: List<Type>> -> Type {
    return Tuple<...Es>;
}

pub pure comptime fn Flags<...flags: List<bool>> -> Type {
    return u32;
}

pub pure comptime fn TypedBag<T: Type, ...items: List<T>> -> Type {
    return T;
}

pub comptime fn uses_user_variadics(
    fs: List<bool>,
    xs: List<bool>,
) -> Type where ...Es: List<Type> {
    const _a: Type = Tagged<true, u8, bool>;
    const _b: Type = Tagged<true, .Es = Es>;
    const _c: Type = Tagged<false, ...Es>;
    const _d: Type = Tagged<true>;
    const _e: Type = Flags<true, false, true>;
    const _f: Type = Flags<.flags = fs>;
    const _g: Type = Flags<...fs>;
    const _h: Type = TypedBag<bool, true, false>;
    const _i: Type = TypedBag<bool, .items = xs>;
    return Tagged<true, ...Es>;
}
"#;
    let package = compile_nir(source, true).expect("user variadic NF heads compile");
    assert!(
        package
            .normal_forms
            .iter()
            .filter(|head| head.role == nir::normal_forms::Role::Opaque)
            .count()
            >= 3
    );
    let _ = compile_options_only;
}

#[test]
fn user_defined_variadic_normal_form_heads_stay_opaque_and_keep_head_identity() {
    let source = r#"
pub const List: pure comptime fn<T: Type> -> Type = .List'builtin;
pub const Tuple: pure comptime fn<...Es: List<Type>> -> Type = .Tuple'builtin;

pub pure comptime fn Wrapper<T: Type> -> Type {
    return T;
}

pub fn cannot_unfold(x: Wrapper<bool>) -> bool {
    return x;
}
"#;
    let err = compile_nir(source, true).expect_err("Wrapper<bool> must not unfold to bool");
    assert!(!err.is_empty());

    let destructure_err = compile_nir(
        r#"
pub const List: pure comptime fn<T: Type> -> Type = .List'builtin;
pub const Tuple: pure comptime fn<...Es: List<Type>> -> Type = .Tuple'builtin;

pub pure comptime fn Bag<...Es: List<Type>> -> Type {
    return Tuple<...Es>;
}

pub fn destructure_opaque(x: Bag<bool, u8>) -> bool {
    const (a, _b) = x;
    return a;
}
"#,
        true,
    )
    .expect_err("opaque variadic head must not destructure as Tuple");
    assert!(!destructure_err.is_empty());
}

#[test]
fn non_type_normal_form_declarations_and_applications_preserve_result_types() {
    let source = r#"
pub pure comptime fn mk_flag<b: bool> -> bool {
    return b;
}

pub pure comptime fn choose_val<T: Type, x: T> -> T {
    return x;
}

pub pure comptime fn use_static_flag() -> bool {
    mk_flag<true>
}

pub pure comptime fn use_static_dep() -> u32 {
    choose_val<u32, 7>
}

pub pure comptime fn apply_flag(f: pure comptime fn<b: bool> -> bool) -> bool {
    f<true>
}

pub pure comptime fn apply_dep(f: pure comptime fn<T: Type, x: T> -> T) -> u32 {
    f<u32, 7>
}
"#;
    let package = compile_nir(source, false).expect("non-Type NF heads compile and validate");

    let use_flag = unwrap_tail(&find_fn(&package, "use_static_flag").body);
    assert_eq!(use_flag.ty, Term::Primitive(P::Bool));
    let K::NormalFormType { .. } = &use_flag.kind else {
        panic!("expected static NormalFormType, got {}", use_flag.kind);
    };

    let use_dep = unwrap_tail(&find_fn(&package, "use_static_dep").body);
    assert_eq!(use_dep.ty, Term::Primitive(P::U32));
    let K::NormalFormType { .. } = &use_dep.kind else {
        panic!(
            "expected dependent static NormalFormType, got {}",
            use_dep.kind
        );
    };

    let apply_flag = unwrap_tail(&find_fn(&package, "apply_flag").body);
    assert_eq!(apply_flag.ty, Term::Primitive(P::Bool));
    let K::ApplyNormalForm { callee, .. } = &apply_flag.kind else {
        panic!("expected dynamic ApplyNormalForm, got {}", apply_flag.kind);
    };
    assert!(callee.ty.is_normal_form_callable());

    let apply_dep = unwrap_tail(&find_fn(&package, "apply_dep").body);
    assert_eq!(apply_dep.ty, Term::Primitive(P::U32));
    let K::ApplyNormalForm { .. } = &apply_dep.kind else {
        panic!("expected dependent ApplyNormalForm, got {}", apply_dep.kind);
    };
}

#[test]
fn non_type_normal_form_applications_reject_type_position_and_result_mismatches() {
    let type_pos_err = compile_nir(
        r#"
pub pure comptime fn mk_flag<b: bool> -> bool {
    return b;
}

pub fn bad(x: mk_flag<true>) -> bool {
    return x;
}
"#,
        false,
    )
    .expect_err("bool-returning NF head cannot be used as a type annotation");
    assert!(!type_pos_err.is_empty());

    let dynamic_mismatch = compile_nir(
        r#"
pub pure comptime fn apply_flag(f: pure comptime fn<b: bool> -> bool) -> u32 {
    return f<true>;
}
"#,
        false,
    )
    .expect_err("bool-returning dynamic NF application must reject u32 return type");
    assert!(!dynamic_mismatch.is_empty());
}
