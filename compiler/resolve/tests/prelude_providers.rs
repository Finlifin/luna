use resolve::{DependencyEdge, build_package_set_with_preludes};
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};
use symbol::Symbol;

#[test]
fn multiple_preludes_are_identity_selected_private_globs_with_self_exclusion() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let sources = [
        "pub mod prelude {pub fn base(){}}",
        "pub mod prelude {pub use core.prelude.base;pub fn metadata(){}}",
        "pub mod prelude {pub fn configuration(){}}",
        "pub mod prelude {pub fn attribute(){}}",
        "pub mod prelude {pub fn specification(){}}",
        "pub mod prelude {pub fn standard(){}}",
        "fn local(){}",
    ];
    // Provider names deliberately differ from the built-in package spellings.
    let mut inputs: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(id, source)| {
            let mut vfs = vfs::Vfs::new(&format!("p{id}"), format!("/p{id}").into());
            let file =
                map.new_source_file(FileName::Custom(format!("p{id}/lib.fl")), (*source).into());
            vfs.selected_root = Some(vfs.add_file("lib.fl".into(), file));
            (id as u32, vfs)
        })
        .collect();
    let dependencies = [DependencyEdge {
        requester: 1,
        alias: "core".into(),
        provider: 0,
    }];
    let mut references: Vec<_> = inputs.iter_mut().map(|(id, vfs)| (*id, vfs)).collect();
    let result = build_package_set_with_preludes(
        &map,
        &diagnostics,
        &mut references,
        &dependencies,
        None,
        &[0, 1, 2, 3, 4],
    )
    .unwrap();
    assert!(!diagnostics.has_errors());
    assert!(result.tree.errors.is_empty(), "{:?}", result.tree.errors);
    let scope = |id| {
        result
            .tree
            .scope_tree
            .get(result.package_scopes[&id])
            .unwrap()
    };
    for name in [
        "base",
        "metadata",
        "configuration",
        "attribute",
        "specification",
    ] {
        let name = Symbol::intern(name);
        assert!(scope(6).items.unique_glob(name).is_some());
        assert!(!scope(6).items.ambiguous_glob(name));
        assert!(scope(6).items.get_local(name).is_none());
        // std is a consumer, but its own prelude is not a default provider.
        assert!(scope(5).items.unique_glob(name).is_some());
    }
    assert!(
        scope(6)
            .items
            .unique_glob(Symbol::intern("standard"))
            .is_none()
    );
    for (id, name) in [
        (1, "metadata"),
        (2, "configuration"),
        (3, "attribute"),
        (4, "specification"),
    ] {
        assert!(scope(id).items.unique_glob(Symbol::intern(name)).is_none());
    }
    for providers in [&[0, 0][..], &[99][..]] {
        assert!(
            build_package_set_with_preludes(
                &map,
                &diagnostics,
                &mut references,
                &dependencies,
                None,
                providers
            )
            .is_err()
        );
    }
}
