//! Rebind a method draft before installing it into a generated implementation.
use super::*;
use hir::common::DefId;
mod attributes;
mod clauses;

/// This is an unpublished draft. Callers must transfer resolver metadata using
/// `bindings`, publish bodies and run ordinary implementation checks. Resolver
/// evidence must be staged using DefinitionBatch::set_resolution.
pub(in crate::program) struct ClonedMember<'h> {
    pub item: hir::Item<'h>,
    pub bodies: Vec<(hir::BodyId, hir::Body<'h>)>,
    pub bindings: HashMap<HirId, HirId>,
    pub nodes: hir::OwnerNodes<'h>,
    pub attributes: Vec<hir::DeferredDefinitionAttribute<'h>>,
    pub resolution: Vec<(HirId, hir::definitions::ResolutionEvidence)>,
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
        signature.clause_params = rebind.clauses(signature.clause_params)?;
        let mut parameters = Vec::new();
        for parameter in signature.params {
            rebind.charge(parameter.span, 0)?;
            if source
                .staged_parameter_definitions
                .contains_key(&parameter.hir_id)
            {
                return Err(error(
                    parameter.span,
                    "generated staged parameter identity publication is not implemented",
                ));
            }
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
        let attributes = rebind.attributes(member)?;
        let resolution = rebind.resolution()?;
        let mut nodes = hir::OwnerNodes::new();
        let signature = match &item.kind {
            ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => signature,
            _ => unreachable!(),
        };
        for parameter in signature.params {
            let parameter = arena.alloc_param(hir::body::BodyParam::from_fn_param(parameter));
            nodes.insert(
                parameter.hir_id.local_id,
                hir::ItemLocalId::new(0),
                hir::Node::Param(parameter),
            );
        }
        for parameter in signature.clause_params {
            nodes.insert(
                parameter.hir_id.local_id,
                hir::ItemLocalId::new(0),
                hir::Node::ClauseParam(parameter),
            );
        }
        Ok((item, resolution, attributes, nodes))
    })();
    *fuel = rebind.budget;
    let (item, resolution, attributes, nodes) = result?;
    Ok(ClonedMember {
        item,
        bodies: rebind.bodies,
        bindings: rebind.bindings,
        resolution,
        attributes,
        nodes,
    })
}

#[cfg(test)]
mod tests;
