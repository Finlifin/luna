//! Closed strategy inputs and locals share one ordered, checked comptime Object.
use super::*;
use hir::common::{DefId, FnParam, Ident};

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
        bindings: rebind.bindings,
        nodes: hir::OwnerNodes::new(),
        attributes: Vec::new(),
        staged_parameters: Vec::new(),
        resolution,
    }
}

pub(in crate::program) fn input_group<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    owner: OwnerId,
    arguments: &[Arg<'h>],
    parameters: &[FnParam<'h>],
    locals: &[&'h Expr<'h>],
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
        member_owner: Some(OwnerId::INVALID),
    };
    let result = (|| {
        rebind.charge(span, 0)?;
        let mut entries = Vec::new();
        let mut statements = Vec::new();
        for (index, (argument, parameter)) in arguments.iter().zip(parameters).enumerate() {
            let Arg::Positional(value) = argument else {
                return Err(error(
                    span,
                    "strategy input requires positional argument matching",
                ));
            };
            let value = rebind.expr(value, 0)?;
            let T::Positional(parameter_type) = parameter.ty.kind else {
                return Err(error(
                    parameter.span,
                    "strategy input requires positional parameter matching",
                ));
            };
            let parameter_type = rebind.expr(parameter_type, 0)?;
            rebind.budget = rebind
                .budget
                .checked_sub(6)
                .ok_or_else(|| error(span, "strategy input binding budget exhausted"))?;
            let base = u32::try_from(index)
                .ok()
                .and_then(|i| i.checked_mul(4))
                .and_then(|i| i.checked_add(10))
                .ok_or_else(|| error(span, "strategy input local identity overflow"))?;
            let binding = synthetic(&mut rebind, base);
            let declaration = synthetic(&mut rebind, base + 1);
            let statement = synthetic(&mut rebind, base + 2);
            let reference = synthetic(&mut rebind, base + 3);
            let name = Ident::new(hir::Symbol::intern(&format!("$input_{index}")), value.span);
            let declaration = arena.alloc_let_decl(hir::LetDecl {
                hir_id: declaration,
                mutable: false,
                comptime: true,
                pattern: hir::Pattern {
                    hir_id: binding,
                    span: value.span,
                    kind: hir::PatternKind::Binding(hir::common::BindingMode::ByValue, name, None),
                },
                guard: None,
                otherwise: None,
                ty: Some(parameter_type),
                init: Some(value),
                span: value.span,
            });
            statements.push(Expr {
                hir_id: statement,
                span: value.span,
                kind: E::Let(declaration),
            });
            rebind
                .captures
                .insert(parameter.hir_id, InputCapture::Local(binding));
            let value = arena.alloc_expr(Expr {
                hir_id: reference,
                span: value.span,
                kind: E::ResolvedLocal(name.name, binding),
            });
            entries.push(hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
                ident: Ident::new(hir::Symbol::intern(&format!("input_{index}")), value.span),
                expr: value,
                span: value.span,
            }));
        }
        for (offset, original) in locals.iter().enumerate() {
            let E::Let(binding) = original.kind else {
                unreachable!()
            };
            let hir::PatternKind::Binding(_, name, _) = binding.pattern.kind else {
                unreachable!()
            };
            let statement = rebind.expr(original, 0)?;
            statements.push(statement.clone());
            let binding_id = rebind.bindings[&binding.pattern.hir_id];
            let index = parameters.len() + offset;
            let synthetic_index = u32::try_from(index)
                .ok()
                .and_then(|i| i.checked_mul(4))
                .and_then(|i| i.checked_add(13))
                .ok_or_else(|| error(original.span, "strategy local identity overflow"))?;
            let reference = synthetic(&mut rebind, synthetic_index);
            rebind.charge(original.span, 0)?;
            let value = arena.alloc_expr(Expr {
                hir_id: reference,
                span: original.span,
                kind: E::ResolvedLocal(name.name, binding_id),
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
        Ok((ty as &_, value as &_, rebind.resolution()?))
    })();
    *fuel = rebind.budget;
    let (ty, value, resolution) = result?;
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
    let T::Positional(ty) = parameter.ty.kind else {
        return Err(error(
            parameter.span,
            "strategy input requires positional parameter matching",
        ));
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
