use super::*;
use hir::{Body, BodyId, HirArena, HirId, Ident, ItemLocalId, Symbol};
use rustc_span::DUMMY_SP;

fn id(owner: OwnerId, index: u32) -> HirId {
    HirId::new(owner, ItemLocalId::new(index))
}

fn expr<'h>(arena: &'h HirArena, owner: OwnerId, index: u32, kind: H<'h>) -> &'h Expr<'h> {
    arena.alloc_expr(Expr {
        hir_id: id(owner, index),
        kind,
        span: DUMMY_SP,
    })
}

fn name() -> Symbol {
    Symbol::intern("same_spelling")
}

fn reference<'h>(arena: &'h HirArena, owner: OwnerId, index: u32, binding: HirId) -> &'h Expr<'h> {
    expr(arena, owner, index, H::ResolvedLocal(name(), binding))
}

struct Fixture<'a, 'h> {
    package: &'a hir::Package<'h>,
    protocols: SourceProtocolIndex<'a, 'h>,
    identities: BTreeMap<u32, u64>,
    types: BTreeMap<OwnerId, Term>,
    containers: BTreeMap<OwnerId, OwnerId>,
}

impl<'a, 'h> Fixture<'a, 'h> {
    fn new(package: &'a hir::Package<'h>) -> Self {
        Self {
            package,
            protocols: SourceProtocolIndex::new(package),
            identities: BTreeMap::new(),
            types: BTreeMap::new(),
            containers: BTreeMap::new(),
        }
    }
    fn reads(&self, owner: OwnerId) -> Reads<'_, 'h> {
        Reads::new(
            Source {
                package: self.package,
                identities: &self.identities,
                self_types: &self.types,
                containers: &self.containers,
                protocols: &self.protocols,
            },
            owner,
            &10_000,
            DUMMY_SP,
        )
    }
}

fn observe(package: &hir::Package<'_>, owner: OwnerId, value: &Expr<'_>) -> BTreeSet<n::LocalId> {
    let fixture = Fixture::new(package);
    let mut reads = fixture.reads(owner);
    reads.query = Query::FreeLocals;
    reads
        .local_graph
        .entry(SourceScope::Expression)
        .or_default();
    reads.expression(value).unwrap();
    reads.drain().unwrap();
    close(
        &reads.local_graph,
        SourceScope::Expression,
        &mut reads.fuel,
        DUMMY_SP,
    )
    .unwrap()
}

#[test]
fn closure_and_postlambda_observe_captures_but_bind_their_own_inputs() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let outside = id(owner, 1);
    let parameter = id(owner, 2);
    let domain = id(owner, 3);
    let domain_use = reference(&arena, owner, 4, domain);
    let capture = reference(&arena, owner, 5, outside);
    let own = reference(&arena, owner, 6, parameter);
    let body_value = expr(&arena, owner, 7, H::Binary(hir::BinOp::And, capture, own));
    let body = BodyId::new(id(owner, 8));
    package.insert_body(
        body,
        Body {
            params: &[],
            value: body_value,
        },
    );
    let parameters = [hir::expr::ClosureParam {
        hir_id: parameter,
        pat: hir::Pattern {
            hir_id: parameter,
            kind: Pattern::Binding(
                hir::BindingMode::ByValue,
                Ident::new(name(), DUMMY_SP),
                None,
            ),
            span: DUMMY_SP,
        },
        ty: Some(domain_use),
        span: DUMMY_SP,
    }];
    let closure = Expr {
        hir_id: id(owner, 9),
        kind: H::Closure(&parameters, None, body),
        span: DUMMY_SP,
    };
    let expected = BTreeSet::from([local(outside), local(domain)]);
    assert_eq!(observe(&package, owner, &closure), expected);
    let callee = expr(&arena, owner, 10, H::Unit);
    let arguments = [Arg::PostLambda(&closure)];
    let call = Expr {
        hir_id: id(owner, 11),
        kind: H::Application(callee, &arguments),
        span: DUMMY_SP,
    };
    assert_eq!(observe(&package, owner, &call), expected);
}

#[test]
fn callable_and_contextual_telescope_binders_do_not_escape() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let outside = id(owner, 1);
    let parameter = id(owner, 2);
    let capture = reference(&arena, owner, 3, outside);
    let own = reference(&arena, owner, 4, parameter);
    let parameters = [hir::common::TyParam::new(
        parameter,
        TyParamKind::PositionalDependencyCatched(Ident::new(name(), DUMMY_SP), capture),
        DUMMY_SP,
    )];
    let head = Expr {
        hir_id: id(owner, 5),
        kind: H::TyFn(&parameters),
        span: DUMMY_SP,
    };
    let arrow = Expr {
        hir_id: id(owner, 6),
        kind: H::TyFnArrow(&head, own),
        span: DUMMY_SP,
    };
    assert_eq!(
        observe(&package, owner, &arrow),
        BTreeSet::from([local(outside)])
    );
    let context = vec![hir::expr::CodeContextBinding {
        hir_id: parameter,
        name: Ident::new(name(), DUMMY_SP),
        ty: capture,
        default: None,
    }];
    let quoted = Expr {
        hir_id: id(owner, 7),
        kind: H::ContextualExprType(context, own),
        span: DUMMY_SP,
    };
    assert_eq!(
        observe(&package, owner, &quoted),
        BTreeSet::from([local(outside)])
    );
}

#[test]
fn referenced_local_declaration_does_not_leak_its_formal_parameters() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let child = package.alloc_owner_id();
    let outside = id(owner, 1);
    let parameter = id(child, 1);
    let ty = expr(&arena, child, 2, H::TyType);
    let own = reference(&arena, child, 3, parameter);
    let capture = reference(&arena, child, 4, outside);
    let body_value = expr(&arena, child, 5, H::Binary(hir::BinOp::And, own, capture));
    let body = BodyId::new(id(child, 6));
    package.insert_body(
        body,
        Body {
            params: &[],
            value: body_value,
        },
    );
    let params = arena.alloc_fn_param_slice(vec![hir::common::FnParam {
        template_annotation: None,
        hir_id: parameter,
        name: Ident::new(name(), DUMMY_SP),
        index: 0,
        ty: hir::common::TyParam::new(parameter, TyParamKind::Positional(ty), DUMMY_SP),
        span: DUMMY_SP,
    }]);
    for (current, params, body) in [(owner, &[][..], None), (child, params, Some(body))] {
        let sig = hir::FnSig {
            params,
            return_ty: None,
            return_bind: None,
            return_binding: None,
            modifiers: Default::default(),
            clause_params: &[],
            clause_constraints: &[],
            span: DUMMY_SP,
        };
        let kind = if let Some(body) = body {
            ItemKind::Fn(sig, body)
        } else {
            ItemKind::FnDeclaration(sig)
        };
        let item = arena.alloc_item(hir::Item {
            owner_id: current,
            ident: Ident::new(name(), DUMMY_SP),
            kind,
            span: DUMMY_SP,
        });
        package.insert_owner(
            current,
            hir::OwnerInfo {
                node: hir::OwnerNode::Item(item),
                nodes: hir::OwnerNodes::new(),
            },
        );
    }
    let definition = hir::common::DefId { pkg: 0, index: 1 };
    package.bind_definition(definition, child);
    package.lexical_parents.insert(child, owner);
    let target = expr(
        &arena,
        owner,
        2,
        H::ResolvedDef {
            name: name(),
            has_clause_param: false,
            def_id: definition,
        },
    );
    assert_eq!(
        observe(&package, owner, target),
        BTreeSet::from([local(outside)])
    );
}

#[test]
fn bindings_created_in_a_region_are_not_inputs_of_its_returned_closure() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let outside = id(owner, 1);
    let alias = id(owner, 2);
    let capture = reference(&arena, owner, 3, outside);
    let binding = arena.alloc_let_decl(hir::LetDecl {
        hir_id: id(owner, 4),
        mutable: false,
        comptime: false,
        pattern: hir::Pattern {
            hir_id: alias,
            kind: Pattern::Binding(
                hir::BindingMode::ByValue,
                Ident::new(name(), DUMMY_SP),
                None,
            ),
            span: DUMMY_SP,
        },
        guard: None,
        otherwise: None,
        ty: None,
        init: Some(capture),
        span: DUMMY_SP,
    });
    let statement = expr(&arena, owner, 5, H::Let(binding));
    let alias_use = reference(&arena, owner, 6, alias);
    let body = BodyId::new(id(owner, 7));
    package.insert_body(
        body,
        Body {
            params: &[],
            value: alias_use,
        },
    );
    let closure = expr(&arena, owner, 8, H::Closure(&[], None, body));
    let statements = arena.alloc_expr_slice(vec![statement.clone()]);
    let block = arena.alloc_block(hir::Block {
        hir_id: id(owner, 9),
        stmts: statements,
        expr: Some(closure),
        span: DUMMY_SP,
    });
    let value = expr(&arena, owner, 10, H::Block(block));
    assert_eq!(
        observe(&package, owner, closure),
        BTreeSet::from([local(alias)])
    );
    assert_eq!(
        observe(&package, owner, value),
        BTreeSet::from([local(outside)])
    );
}

#[test]
fn declaration_queries_keep_enclosing_inputs_and_exclude_callee_binders() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let child = package.alloc_owner_id();
    let outer = hir::common::DefId { pkg: 0, index: 10 };
    let inner = hir::common::DefId { pkg: 0, index: 11 };
    let definition = hir::common::DefId { pkg: 0, index: 12 };
    let capture = expr(&arena, child, 2, H::ResolvedClauseParam(name(), outer));
    let own = expr(&arena, child, 3, H::ResolvedClauseParam(name(), inner));
    let body_value = expr(&arena, child, 4, H::Binary(hir::BinOp::Eq, capture, own));
    let body = BodyId::new(id(child, 5));
    package.insert_body(
        body,
        Body {
            params: &[],
            value: body_value,
        },
    );
    for (current, input, body) in [(owner, outer, None), (child, inner, Some(body))] {
        let ident = Ident::new(name(), DUMMY_SP);
        let params = arena.alloc_clause_param_slice(vec![hir::ClauseParam {
            hir_id: id(current, 1),
            definition: Some(input),
            name: ident,
            kind: Clause::Type(ident),
            span: DUMMY_SP,
        }]);
        let sig = hir::FnSig {
            params: &[],
            return_ty: None,
            return_bind: None,
            return_binding: None,
            modifiers: Default::default(),
            clause_params: params,
            clause_constraints: &[],
            span: DUMMY_SP,
        };
        let item = arena.alloc_item(hir::Item {
            owner_id: current,
            ident,
            span: DUMMY_SP,
            kind: if let Some(body) = body {
                ItemKind::Fn(sig, body)
            } else {
                ItemKind::FnDeclaration(sig)
            },
        });
        package.insert_owner(
            current,
            hir::OwnerInfo {
                node: hir::OwnerNode::Item(item),
                nodes: hir::OwnerNodes::new(),
            },
        );
    }
    package.bind_definition(definition, child);
    package.lexical_parents.insert(child, owner);
    let target = expr(
        &arena,
        owner,
        2,
        H::ResolvedDef {
            name: name(),
            has_clause_param: true,
            def_id: definition,
        },
    );
    {
        let fixture = Fixture::new(&package);
        let mut reads = fixture.reads(owner);
        reads.query = Query::FreeDeclarations;
        reads.graph.entry(SourceScope::Expression).or_default();
        reads.expression(target).unwrap();
        reads.drain().unwrap();
        let expected = BTreeSet::from([reads.identity(outer)]);
        assert_eq!(
            close(
                &reads.graph,
                SourceScope::Expression,
                &mut reads.fuel,
                DUMMY_SP
            )
            .unwrap(),
            expected
        );
        // Ordinary declaration formation binds its own telescope. The same
        // query over an expression must keep the enclosing telescope external.
        assert_eq!(
            close(
                &reads.graph,
                SourceScope::Declaration(child),
                &mut reads.fuel,
                DUMMY_SP
            )
            .unwrap(),
            expected
        );
    }
    let own_input = expr(&arena, owner, 3, H::ResolvedClauseParam(name(), outer));
    {
        let fixture = Fixture::new(&package);
        let mut reads = fixture.reads(owner);
        reads.query = Query::FreeDeclarations;
        reads.graph.entry(SourceScope::Expression).or_default();
        reads.expression(own_input).unwrap();
        reads.drain().unwrap();
        assert_eq!(
            close(
                &reads.graph,
                SourceScope::Expression,
                &mut reads.fuel,
                DUMMY_SP
            )
            .unwrap(),
            BTreeSet::from([reads.identity(outer)])
        );
    }
}

#[test]
fn exact_declaration_queries_reject_unmapped_delivered_scopes() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let value = expr(
        &arena,
        owner,
        1,
        H::DeliveredType(std::sync::Arc::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        })),
    );
    for query in [Query::FreeDeclarations, Query::FreeInputs] {
        let fixture = Fixture::new(&package);
        let mut reads = fixture.reads(owner);
        reads.query = query;
        reads.expression(value).unwrap();
        assert!(
            reads
                .drain()
                .unwrap_err()
                .to_string()
                .contains("requires a delivered scope mapping")
        );
    }
}

#[test]
fn readiness_inputs_separate_clause_supply_from_staged_local_supply() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let owner = package.alloc_owner_id();
    let binding = id(owner, 1);
    let staged = hir::common::DefId { pkg: 0, index: 10 };
    let clause = hir::common::DefId { pkg: 0, index: 11 };
    package.staged_parameter_definitions.insert(binding, staged);
    let local_use = expr(&arena, owner, 2, H::ResolvedFnParam(name(), binding));
    let clause_use = expr(&arena, owner, 3, H::ResolvedClauseParam(name(), clause));
    let transported = expr(&arena, owner, 4, H::ResolvedClauseParam(name(), staged));
    let fields = [local_use.clone(), clause_use.clone(), transported.clone()];
    let value = expr(&arena, owner, 5, H::Tuple(&fields));
    let fixture = Fixture::new(&package);
    let mut failed = fixture.reads(owner);
    failed.query = Query::FreeInputs;
    failed.fuel = 0;
    assert!(failed.expression(value).is_err());
    let mut reads = fixture.reads(owner);
    reads.query = Query::FreeInputs;
    reads.expression(value).unwrap();
    reads.drain().unwrap();
    let expected = SourceInputs {
        locals: BTreeSet::from([local(binding)]),
        declarations: BTreeSet::from([reads.identity(clause)]),
    };
    let observed = reads.free_inputs().unwrap();
    assert_eq!(observed, expected);
    assert_eq!(
        observed.to_string(),
        format!(
            "(source-inputs (locals {}) (declarations {}))",
            local(binding),
            reads.identity(clause)
        )
    );
}
