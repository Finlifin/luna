//! Closed strategy inputs and locals share one ordered, checked comptime Object.
use super::*;
use hir::common::{DefId, FnParam, Ident};

/// One successful pattern output; its type is inferred independently of siblings.
pub(in crate::program) struct GenerationBinding<'h> {
    pub id: HirId,
    pub name: Ident,
    pub annotation: Option<&'h Expr<'h>>,
    pub span: rustc_span::Span,
}

fn synthetic(rebind: &mut Rebind<'_, '_>, index: u32) -> HirId {
    rebind.id(HirId::new(OwnerId::INVALID, hir::ItemLocalId::new(index)))
}
fn finish<'h>(
    rebind: Rebind<'_, 'h>,
    name: &str,
    span: rustc_span::Span,
    ty: &'h Expr<'h>,
    value: &'h Expr<'h>,
    resolution: Vec<(HirId, hir::definitions::ResolutionEvidence)>,
) -> ClonedMember<'h> {
    ClonedMember {
        item: hir::Item {
            owner_id: rebind.namespace,
            ident: Ident::new(hir::Symbol::intern(name), span),
            span,
            kind: ItemKind::Const(ty, value),
        },
        bodies: rebind.bodies,
        capture_uses: rebind.capture_uses,
        bindings: rebind.bindings,
        nodes: hir::OwnerNodes::new(),
        attributes: Vec::new(),
        staged_parameters: Vec::new(),
        resolution,
    }
}

pub(in crate::program) fn execution_draft<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    owner: OwnerId,
    strategy: OwnerId,
    subject: &'h Expr<'h>,
    parameters: &[FnParam<'h>],
    constraints: &[hir::clause::ClauseConstraint<'h>],
    input_stages: &HashMap<HirId, DefId>,
    locals: &[GenerationBinding<'h>],
    computations: &[&'h Expr<'h>],
    exported: &BTreeSet<usize>,
    definitions: &HashMap<DefId, DefId>,
    span: rustc_span::Span,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    *fuel = fuel
        .checked_sub(definitions.len())
        .ok_or_else(|| error(span, "strategy body identity budget exhausted"))?;
    let mut rebind = Rebind {
        source,
        arena,
        namespace: owner,
        bindings: HashMap::new(),
        budget: *fuel,
        bodies: Vec::new(),
        definitions: definitions.clone(),
        captures: HashMap::new(),
        capture_uses: BTreeSet::new(),
        self_type: Some(subject),
        member_owner: Some(strategy),
    };
    let result = (|| {
        rebind.charge(span, 0)?;
        let mut entries = Vec::new();
        let mut statements = Vec::new();
        let mut cloned_parameters = Vec::new();
        let mut staged_parameters = Vec::new();
        for (index, parameter) in parameters.iter().enumerate() {
            rebind.charge(parameter.span, 0)?;
            let mut cloned = parameter.clone();
            cloned.hir_id = rebind.id(parameter.hir_id);
            cloned.ty = rebind.ty(&parameter.ty, 0)?;
            cloned.ty.flags |= hir::common::TPARAM_COMPTIME;
            let identity = input_stages
                .get(&parameter.hir_id)
                .copied()
                .ok_or_else(|| {
                    error(
                        parameter.span,
                        "strategy input has no fresh staged identity",
                    )
                })?;
            staged_parameters.push((cloned.hir_id, identity));
            let reference = synthetic(&mut rebind, 10 + index as u32);
            let value = arena.alloc_expr(Expr {
                hir_id: reference,
                span: parameter.span,
                kind: if exported.contains(&index) {
                    E::ResolvedFnParam(parameter.name.name, cloned.hir_id)
                } else {
                    E::Unit
                },
            });
            cloned_parameters.push(cloned);
            entries.push(hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
                ident: Ident::new(hir::Symbol::intern(&format!("input_{index}")), value.span),
                expr: value,
                span: value.span,
            }));
        }
        for computation in computations {
            statements.push(rebind.expr(computation, 0)?.clone());
        }
        for (offset, original) in locals.iter().enumerate() {
            let name = original.name;
            let binding_id = rebind.bindings[&original.id];
            let index = parameters.len() + offset;
            let synthetic_index = u32::try_from(index)
                .ok()
                .and_then(|i| i.checked_add(10))
                .ok_or_else(|| error(original.span, "strategy local identity overflow"))?;
            let reference = synthetic(&mut rebind, synthetic_index);
            rebind.charge(original.span, 0)?;
            let value = arena.alloc_expr(Expr {
                hir_id: reference,
                span: original.span,
                kind: if exported.contains(&index) {
                    E::ResolvedLocal(name.name, binding_id)
                } else {
                    E::Unit
                },
            });
            entries.push(hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
                ident: Ident::new(
                    hir::Symbol::intern(&format!("input_{index}")),
                    original.span,
                ),
                expr: value,
                span: original.span,
            }));
        }
        let ty_id = synthetic(&mut rebind, 0);
        let value_id = synthetic(&mut rebind, 1);
        let ty = arena.alloc_expr(Expr {
            hir_id: ty_id,
            span,
            kind: E::Builtin(hir::Symbol::intern("Object")),
        });
        let object_id = synthetic(&mut rebind, 2);
        let block_id = synthetic(&mut rebind, 3);
        let object = arena.alloc_expr(Expr {
            hir_id: object_id,
            span,
            kind: E::Object(entries),
        });
        let block = arena.alloc_block(hir::Block {
            hir_id: block_id,
            span,
            stmts: arena.alloc_expr_slice(statements),
            expr: Some(object),
        });
        let value = arena.alloc_expr(Expr {
            hir_id: value_id,
            span,
            kind: E::Block(block),
        });
        let constraints = rebind.constraints(constraints)?;
        Ok((
            ty as &_,
            value as &_,
            rebind.resolution()?,
            cloned_parameters,
            staged_parameters,
            constraints,
        ))
    })();
    *fuel = rebind.budget;
    let (ty, value, resolution, parameters, staged_parameters, constraints) = result?;
    let body_id = hir::BodyId::new(synthetic(&mut rebind, 4));
    let parameters = arena.alloc_fn_param_slice(parameters);
    let mut draft = finish(rebind, "$derive_execute", span, ty, value, resolution);
    draft.item.kind = ItemKind::Fn(
        hir::item::FnSig {
            params: parameters,
            return_ty: Some(ty),
            return_bind: None,
            return_binding: None,
            modifiers: hir::item::FnModifiers {
                is_comptime: true,
                ..Default::default()
            },
            clause_params: &[],
            clause_constraints: constraints,
            span,
        },
        body_id,
    );
    draft.bodies.push((
        body_id,
        hir::Body {
            params: arena
                .alloc_param_slice(parameters.iter().map(hir::body::BodyParam::from_fn_param)),
            value,
        },
    ));
    for parameter in parameters {
        let parameter = arena.alloc_param(hir::body::BodyParam::from_fn_param(parameter));
        draft.nodes.insert(
            parameter.hir_id.local_id,
            hir::ItemLocalId::new(0),
            hir::Node::Param(parameter),
        );
    }
    draft.staged_parameters = staged_parameters;
    Ok(draft)
}

pub(in crate::program) fn input_group<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    owner: OwnerId,
    execution: DefId,
    arguments: &[Arg<'h>],
    extended: bool,
    span: rustc_span::Span,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    let mut rebind = Rebind {
        source,
        arena,
        namespace: owner,
        bindings: HashMap::new(),
        budget: *fuel,
        bodies: Vec::new(),
        definitions: HashMap::new(),
        captures: HashMap::new(),
        capture_uses: BTreeSet::new(),
        self_type: None,
        member_owner: Some(OwnerId::INVALID),
    };
    let head = arena.alloc_expr(Expr {
        hir_id: HirId::new(OwnerId::INVALID, hir::ItemLocalId::new(0)),
        span,
        kind: E::ResolvedDef {
            name: hir::Symbol::intern("$derive_execute"),
            def_id: execution,
            has_clause_param: false,
        },
    });
    let call = arena.alloc_expr(Expr {
        hir_id: HirId::new(OwnerId::INVALID, hir::ItemLocalId::new(1)),
        span,
        kind: if extended {
            E::ExtendedApplication(head, arena.alloc_arg_slice(arguments.to_vec()))
        } else {
            E::Application(head, arena.alloc_arg_slice(arguments.to_vec()))
        },
    });
    let result = rebind.expr(call, 0);
    *fuel = rebind.budget;
    let value = result?;
    let ty_id = synthetic(&mut rebind, 2);
    let ty = arena.alloc_expr(Expr {
        hir_id: ty_id,
        span,
        kind: E::Builtin(hir::Symbol::intern("Object")),
    });
    let resolution = rebind.resolution()?;
    *fuel = rebind.budget;
    Ok(finish(
        rebind,
        "$derive_inputs",
        span,
        ty,
        value,
        resolution,
    ))
}

pub(in crate::program) fn input_draft<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    owner: OwnerId,
    parameter: &FnParam<'h>,
    group: DefId,
    index: usize,
    definitions: &HashMap<DefId, DefId>,
    captures: &HashMap<HirId, InputCapture>,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    let ty = match parameter.ty.kind {
        T::Positional(ty) | T::Optional(_, ty, _) | T::Varadic(_, ty) => ty,
        _ => {
            return Err(error(
                parameter.span,
                "unsupported strategy parameter shape",
            ));
        }
    };
    value_draft(
        source,
        arena,
        owner,
        Some(ty),
        parameter.span,
        group,
        index,
        definitions,
        captures,
        fuel,
    )
}

pub(in crate::program) fn value_draft<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    owner: OwnerId,
    annotation: Option<&'h Expr<'h>>,
    span: rustc_span::Span,
    group: DefId,
    index: usize,
    definitions: &HashMap<DefId, DefId>,
    captures: &HashMap<HirId, InputCapture>,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    *fuel = fuel
        .checked_sub(definitions.len().saturating_add(captures.len()))
        .ok_or_else(|| error(span, "strategy input identity budget exhausted"))?;
    let mut rebind = Rebind {
        source,
        arena,
        namespace: owner,
        bindings: HashMap::new(),
        budget: *fuel,
        bodies: Vec::new(),
        definitions: definitions.clone(),
        captures: captures.clone(),
        capture_uses: BTreeSet::new(),
        self_type: None,
        member_owner: Some(OwnerId::INVALID),
    };
    let result = (|| {
        let ty = match annotation {
            Some(ty) => rebind.expr(ty, 0)?,
            None => {
                let id = synthetic(&mut rebind, 2);
                arena.alloc_expr(Expr {
                    hir_id: id,
                    span,
                    kind: E::TyPlaceholder,
                })
            }
        };
        rebind.charge(span, 0)?;
        let group_id = synthetic(&mut rebind, 0);
        let value_id = synthetic(&mut rebind, 1);
        let group = arena.alloc_expr(Expr {
            hir_id: group_id,
            span: span,
            kind: E::ResolvedDef {
                name: hir::Symbol::intern("$derive_inputs"),
                def_id: group,
                has_clause_param: false,
            },
        });
        let value = arena.alloc_expr(Expr {
            hir_id: value_id,
            span: span,
            kind: E::ConstantObjectEntry(group, index),
        });
        Ok((ty, value as &_, rebind.resolution()?))
    })();
    *fuel = rebind.budget;
    let (ty, value, resolution) = result?;
    Ok(finish(
        rebind,
        &format!("$derive_input_{index}"),
        span,
        ty,
        value,
        resolution,
    ))
}

/// Cache the implementation's subject identity instead of re-evaluating its expression.
pub(in crate::program) fn subject_draft<'h>(
    arena: &'h HirArena,
    owner: OwnerId,
    span: rustc_span::Span,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    *fuel = fuel
        .checked_sub(3)
        .ok_or_else(|| error(span, "strategy subject capture budget exhausted"))?;
    let ty = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, hir::ItemLocalId::new(1)),
        span,
        kind: E::TyType,
    });
    let value = arena.alloc_expr(Expr {
        hir_id: HirId::new(owner, hir::ItemLocalId::new(2)),
        span,
        kind: E::TySelf,
    });
    Ok(ClonedMember {
        item: hir::Item {
            owner_id: owner,
            ident: Ident::new(hir::Symbol::intern("$derive_subject"), span),
            span,
            kind: ItemKind::Const(ty, value),
        },
        bodies: Vec::new(),
        bindings: HashMap::new(),
        nodes: hir::OwnerNodes::new(),
        attributes: Vec::new(),
        staged_parameters: Vec::new(),
        resolution: Vec::new(),
        capture_uses: BTreeSet::new(),
    })
}
