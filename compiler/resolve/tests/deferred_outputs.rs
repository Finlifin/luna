use resolve::binding::{Resolution, Visibility};
use resolve::deferred::OutputBinding;
use resolve::error::ResolveError;
use resolve::ids::{AstNodeRef, DefId};
use resolve::scope::{Scope, ScopeKind, ScopeTree};
use resolve::{Binding, BindingKind, ModuleTree, Resolver, Rib, RibKind, ScopeId};
use rustc_span::DUMMY_SP;
use symbol::Symbol;

fn source(node: u32) -> AstNodeRef {
    AstNodeRef::new(vfs::FileId::from_raw(7), node)
}
fn local(node: u32) -> Binding {
    Binding {
        kind: BindingKind::Local(source(node)),
        vis: Visibility::Private,
        entry: None,
    }
}
fn output(node: u32) -> Resolution {
    Resolution {
        kind: BindingKind::ExpandedLocal(source(node), 0),
    }
}
fn tree() -> ModuleTree {
    let mut scopes = ScopeTree::new();
    scopes.add_scope(Scope::new(
        ScopeId::ROOT,
        ScopeKind::Package,
        None,
        None,
        DefId::new(0, 0),
        false,
    ));
    ModuleTree {
        scope_tree: scopes,
        def_names: Default::default(),
        def_count: 1,
        impls: vec![],
        errors: vec![],
        file_scopes: Default::default(),
    }
}
fn pending(resolver: &mut Resolver<'_>, node: u32) {
    let mut rib = Rib::new(RibKind::MatchBody, ScopeId::INVALID);
    rib.pending_outputs = Some(source(node));
    resolver.ribs_mut().push(rib);
}

#[test]
fn pending_is_not_missing_and_lookup_resumes_in_lexical_order() {
    let tree = tree();
    let mut resolver = Resolver::new(&tree);
    let name = Symbol::intern("value");
    let mut outer = Rib::new(RibKind::Block, ScopeId::INVALID);
    outer.define(name, local(1));
    resolver.ribs_mut().push(outer);
    pending(&mut resolver, 2);
    pending(&mut resolver, 3);
    let plan = resolver.resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP);
    assert_eq!(plan.pending_outputs, vec![source(3), source(2)]);
    assert!(resolver.ribs().lookup(name).is_none());
    assert!(
        resolver
            .resolve_name(name, ScopeId::ROOT, DUMMY_SP)
            .unwrap_err()
            .message()
            .contains("awaits")
    );
    let mut visited = vec![];
    assert_eq!(
        plan.resolve_with(|site, queried| {
            assert_eq!(queried, name);
            visited.push(site);
            Ok(OutputBinding::Pending)
        })
        .unwrap(),
        None
    );
    assert_eq!(visited, vec![source(3)]);
    let resolved = plan
        .resolve_with(|site, _| {
            Ok(if site == source(3) {
                OutputBinding::Missing
            } else {
                OutputBinding::Resolved(output(2))
            })
        })
        .unwrap();
    assert_eq!(resolved, Some(output(2)));
    assert_eq!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Missing))
            .unwrap(),
        Some(Resolution::from_binding(&local(1)))
    );
}

#[test]
fn failed_output_query_does_not_consume_the_plan() {
    let tree = tree();
    let mut resolver = Resolver::new(&tree);
    let name = Symbol::intern("value");
    pending(&mut resolver, 2);
    let plan = resolver.resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP);
    assert!(
        plan.resolve_with(|_, _| Err(ResolveError::InternalError("failed generator".into())))
            .is_err()
    );
    assert_eq!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Resolved(output(2))))
            .unwrap(),
        Some(output(2))
    );
    assert!(matches!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Missing)),
        Err(ResolveError::UnresolvedIdentifier { .. })
    ));
    assert_eq!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Pending))
            .unwrap(),
        None
    );
}

#[test]
fn later_bindings_shadow_pending_outputs_and_scope_exit_restores_outer_names() {
    let tree = tree();
    let mut resolver = Resolver::new(&tree);
    let name = Symbol::intern("value");
    let mut outer = Rib::new(RibKind::Block, ScopeId::INVALID);
    outer.define(name, local(1));
    resolver.ribs_mut().push(outer);
    pending(&mut resolver, 2);
    let mut later = Rib::new(RibKind::MatchBody, ScopeId::INVALID);
    later.define(name, local(3));
    resolver.ribs_mut().push(later);
    let plan = resolver.resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP);
    assert!(plan.pending_outputs.is_empty());
    assert_eq!(
        plan.resolve_with(|_, _| panic!("shadowed scope must not be queried"))
            .unwrap(),
        Some(Resolution::from_binding(&local(3)))
    );
    resolver.ribs_mut().pop();
    assert_eq!(
        resolver
            .resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP)
            .pending_outputs,
        vec![source(2)]
    );
    resolver.ribs_mut().pop();
    assert_eq!(
        resolver
            .resolve_name(name, ScopeId::ROOT, DUMMY_SP)
            .unwrap(),
        Resolution::from_binding(&local(1))
    );
    resolver
        .ribs_mut()
        .push(Rib::new(RibKind::Block, ScopeId::INVALID));
    assert!(
        resolver
            .resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP)
            .pending_outputs
            .is_empty()
    );
}

#[test]
fn an_inner_block_declaration_precedes_an_outer_pending_scope() {
    let mut tree = tree();
    let name = Symbol::intern("value");
    let mut block = Scope::new(
        ScopeId::new(1),
        ScopeKind::Block,
        Some(ScopeId::ROOT),
        None,
        DefId::new(0, 1),
        true,
    );
    let declaration = Binding {
        kind: BindingKind::Definition(DefId::new(0, 2), ScopeId::new(1)),
        vis: Visibility::Public,
        entry: None,
    };
    block.items.define(name, declaration.clone()).unwrap();
    tree.scope_tree.add_scope(block);
    let mut resolver = Resolver::new(&tree);
    pending(&mut resolver, 3);
    resolver
        .ribs_mut()
        .push(Rib::new(RibKind::Block, ScopeId::new(1)));
    let plan = resolver.resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP);
    assert!(plan.pending_outputs.is_empty());
    assert_eq!(
        plan.fallback.unwrap(),
        Resolution::from_binding(&declaration)
    );
}

#[test]
fn deferred_fallback_preserves_access_errors() {
    let mut tree = tree();
    let name = Symbol::intern("secret");
    tree.scope_tree.add_scope(Scope::new(
        ScopeId::new(1),
        ScopeKind::Module,
        Some(ScopeId::ROOT),
        None,
        DefId::new(0, 1),
        false,
    ));
    tree.scope_tree
        .get_mut(ScopeId::ROOT)
        .unwrap()
        .items
        .define(
            name,
            Binding {
                kind: BindingKind::Definition(DefId::new(0, 2), ScopeId::new(1)),
                vis: Visibility::Private,
                entry: None,
            },
        )
        .unwrap();
    let mut resolver = Resolver::new(&tree);
    let original = resolver
        .resolve_name(name, ScopeId::ROOT, DUMMY_SP)
        .unwrap_err()
        .message();
    pending(&mut resolver, 3);
    let plan = resolver.resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP);
    assert_eq!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Pending))
            .unwrap(),
        None
    );
    assert_eq!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Missing))
            .unwrap_err()
            .message(),
        original
    );
    // A real nearer local can shadow the inaccessible outer entry; no access
    // to that entry is granted and it remains the fallback on Missing.
    assert_eq!(
        plan.resolve_with(|_, _| Ok(OutputBinding::Resolved(output(3))))
            .unwrap(),
        Some(output(3))
    );
}

#[test]
fn lookup_dump_is_stable_escaped_sexpression() {
    let tree = tree();
    let mut resolver = Resolver::new(&tree);
    let name = Symbol::intern("value\"\n");
    let mut outer = Rib::new(RibKind::Block, ScopeId::INVALID);
    outer.define(name, local(1));
    resolver.ribs_mut().push(outer);
    pending(&mut resolver, 2);
    let plan = resolver.resolve_name_deferred(name, ScopeId::ROOT, DUMMY_SP);
    assert_eq!(
        plan.to_string(),
        "(name-resolution (name \"value\\\"\\n\") (pending (source 7 2)) (fallback (local (source 7 1))))"
    );
    assert_eq!(plan.to_string(), plan.clone().to_string());
    assert_eq!(
        OutputBinding::Pending.to_string(),
        "(output-binding pending)"
    );
    assert_eq!(
        OutputBinding::Missing.to_string(),
        "(output-binding missing)"
    );
    assert_eq!(
        OutputBinding::Resolved(output(2)).to_string(),
        "(output-binding (expanded-local (source 7 2) (output 0)))"
    );
}
