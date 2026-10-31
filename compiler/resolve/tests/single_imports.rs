use resolve::build_package_set;
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};
use symbol::Symbol;

fn check(source: &str, accepted: bool, equal: Option<(&str, &str, &str)>) {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut package = vfs::Vfs::new("app", "/app".into());
    let file = map.new_source_file(FileName::Custom("app/lib.fl".into()), source.into());
    package.selected_root = Some(package.add_file("lib.fl".into(), file));
    let result = build_package_set(&map, &diagnostics, &mut [(1, &mut package)], &[]).unwrap();
    assert_eq!(
        result.tree.errors.is_empty(),
        accepted,
        "{source}\n{:?}",
        result.tree.errors
    );
    assert!(!diagnostics.has_errors(), "fixture must parse: {source}");
    if let Some((scope, left, right)) = equal {
        let scope = result
            .tree
            .scope_tree
            .iter()
            .find(|s| s.name == Some(Symbol::intern(scope)))
            .unwrap();
        let left = scope.items.get_local(Symbol::intern(left)).unwrap();
        let right = scope.items.get_local(Symbol::intern(right)).unwrap();
        assert_eq!(left.kind, right.kind);
    }
}

#[test]
fn bare_imports_find_parent_declarations_and_preserve_alias_identity() {
    check(
        "pub mod math {} mod prelude {pub use math; use .math as expected;}",
        true,
        Some(("prelude", "math", "expected")),
    );
    check(
        "pub mod math {} mod outer {mod inner {use math as alias; use ..math as expected;}}",
        true,
        Some(("inner", "alias", "expected")),
    );
    check(
        "pub mod math {} mod prelude {pub use math; pub use math;}",
        true,
        None,
    );
}

#[test]
fn nearest_declarations_and_later_explicit_imports_win() {
    check(
        "pub mod math {} mod inner {mod math {} use math as alias; use .inner.math as expected;}",
        true,
        Some(("inner", "alias", "expected")),
    );
    for imports in [
        "use math as alias; use other.math;",
        "use other.math; use math as alias;",
    ] {
        check(
            &format!(
                "mod math {{}} mod other {{pub mod math {{}}}} mod inner {{{imports} use other.math as expected;}}"
            ),
            true,
            Some(("inner", "alias", "expected")),
        );
    }
}

#[test]
fn glob_shadowing_is_independent_of_import_order() {
    for imports in [
        "use math as alias; use other.*;",
        "use other.*; use math as alias;",
    ] {
        check(
            &format!(
                "mod math {{}} mod other {{pub mod math {{}}}} mod inner {{{imports} use other.math as expected;}}"
            ),
            true,
            Some(("inner", "alias", "expected")),
        );
    }
    check(
        "mod math {} mod a {pub mod math {}} mod b {pub mod math {}} mod inner {use a.*;use b.*;use math as alias;}",
        false,
        None,
    );
}

#[test]
fn transitive_globs_wait_for_later_module_imports() {
    for imports in [
        "use bridge.*; use math as alias;",
        "use math as alias; use bridge.*;",
    ] {
        check(
            &format!(
                "mod math {{}} mod inner {{{imports} use source.math as expected;}} mod bridge {{pub use source.*;}} mod source {{pub mod math {{}}}}"
            ),
            true,
            Some(("inner", "alias", "expected")),
        );
    }
}

#[test]
fn explicit_anchors_and_visibility_do_not_fall_back() {
    check(
        "pub mod math {} mod outer {mod inner {use .math;}}",
        false,
        None,
    );
    check(
        "mod outer {private mod math {} pub mod inner {pub use math;}}",
        false,
        None,
    );
    check("mod math {} mod inner {pub use math;}", false, None);
}

#[test]
fn cycles_without_declarations_cannot_manufacture_sources() {
    check("mod inner {use x as y;use y as x;}", false, None);
    check("mod inner {use x;}", false, None);
    check("mod x {} mod inner {use y as x; use x as y;}", false, None);
}

#[test]
fn qualified_imports_wait_for_nearer_prefix_sources() {
    for imports in [
        "use math.value as alias; use provider.math;",
        "use provider.math; use math.value as alias;",
        "use math.value as alias; use provider.*;",
        "use provider.*; use math.value as alias;",
    ] {
        check(
            &format!(
                "mod math {{pub fn value() {{}}}} mod inner {{{imports} use provider.math.value as expected;}} mod provider {{pub mod math {{pub fn value() {{}}}}}}"
            ),
            true,
            Some(("inner", "alias", "expected")),
        );
    }
}

#[test]
fn ambiguous_prefixes_cannot_fall_back_to_an_outer_container() {
    check(
        "mod math {pub fn value() {}} mod a {pub mod math {pub fn value() {}}} mod b {pub mod math {pub fn value() {}}} mod inner {use a.*;use b.*;use math.value as alias;}",
        false,
        None,
    );
}

#[test]
fn qualified_paths_keep_member_boundaries_after_selecting_the_prefix() {
    check(
        "pub fn value() {} mod math {} mod inner {use math.value;}",
        false,
        None,
    );
    check(
        "mod math {pub fn value() {}} mod inner {mod math {} use math.value;}",
        false,
        None,
    );
    check(
        "mod outer {private mod math {pub fn value() {}}} mod math {pub fn value() {}} mod inner {use outer.math.value;}",
        false,
        None,
    );
}
