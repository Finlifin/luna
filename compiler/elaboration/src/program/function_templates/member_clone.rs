//! Rebind a method draft before installing it into a generated implementation.
use super::*;
use hir::common::DefId;

/// This is an unpublished draft. Callers must transfer resolver metadata using
/// `bindings`, publish bodies and run ordinary implementation checks.
pub(in crate::program) struct ClonedMember<'h> {
    pub item: hir::Item<'h>,
    pub bodies: Vec<(hir::BodyId, hir::Body<'h>)>,
    pub bindings: HashMap<HirId, HirId>,
}

pub(in crate::program) fn clone_member<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    member: OwnerId,
    owner: OwnerId,
    definitions: &HashMap<DefId, DefId>,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    let item = source
        .item(member)
        .ok_or_else(|| error(rustc_span::DUMMY_SP, "missing generated member source"))?;
    if owner == member || source.owner(owner).is_some() {
        return Err(error(
            item.span,
            "generated member needs an unoccupied fresh owner",
        ));
    }
    let (signature, body) = match &item.kind {
        ItemKind::Fn(signature, body) => (signature, Some(*body)),
        ItemKind::FnDeclaration(signature) => (signature, None),
        _ => {
            return Err(error(
                item.span,
                "generated method draft requires a function member",
            ));
        }
    };
    if !signature.clause_params.is_empty() {
        return Err(error(
            item.span,
            "generated method clause identities require substitution support",
        ));
    }
    *fuel = fuel
        .checked_sub(definitions.len())
        .ok_or_else(|| error(item.span, "generated member identity budget exhausted"))?;
    let mut rebind = Rebind {
        source,
        arena,
        namespace: owner,
        bindings: HashMap::new(),
        budget: *fuel,
        bodies: Vec::new(),
        definitions: definitions.clone(),
        member_owner: Some(member),
    };
    let result = (|| {
        rebind.charge(item.span, 0)?;
        let mut signature = signature.clone();
        let mut parameters = Vec::new();
        for parameter in signature.params {
            rebind.charge(parameter.span, 0)?;
            let mut parameter = parameter.clone();
            parameter.hir_id = rebind.id(parameter.hir_id);
            parameter.ty = rebind.ty(&parameter.ty, 0)?;
            parameters.push(parameter);
        }
        signature.params = arena.alloc_fn_param_slice(parameters);
        signature.return_ty = signature
            .return_ty
            .map(|value| rebind.expr(value, 0))
            .transpose()?;
        signature.return_binding = signature.return_binding.map(|id| rebind.id(id));
        let mut constraints = Vec::new();
        for constraint in signature.clause_constraints {
            rebind.charge(constraint.span, 0)?;
            let mut constraint = constraint.clone();
            constraint.hir_id = rebind.id(constraint.hir_id);
            constraint.kind = match constraint.kind {
                C::Requires(value) => C::Requires(rebind.expr(value, 0)?),
                C::Ensures(value) => C::Ensures(rebind.expr(value, 0)?),
                C::Decreases(value) => C::Decreases(rebind.expr(value, 0)?),
                C::Outcome => C::Outcome,
            };
            constraints.push(constraint);
        }
        signature.clause_constraints = arena.alloc_clause_slice(constraints);
        let body = body
            .map(|body| rebind.body(body, item.span, 0))
            .transpose()?;
        let mut item = item.clone();
        item.owner_id = owner;
        item.kind = match body {
            Some(body) => ItemKind::Fn(signature, body),
            None => ItemKind::FnDeclaration(signature),
        };
        Ok(item)
    })();
    *fuel = rebind.budget;
    let item = result?;
    Ok(ClonedMember {
        item,
        bodies: rebind.bodies,
        bindings: rebind.bindings,
    })
}

#[cfg(test)]
mod tests;
