//! Rebind a method or associated member draft before installing it into a generated implementation.
use super::*;
use hir::common::DefId;
mod attributes;
mod clauses;

/// This is an unpublished draft. Callers must transfer resolver metadata using
/// `bindings`, publish bodies and run ordinary implementation checks. Resolver
/// evidence must be staged using DefinitionBatch::set_resolution.
pub(in crate::program) struct ClonedMember<'h> {
    pub capture_uses: BTreeSet<DefId>,
    pub item: hir::Item<'h>,
    pub bodies: Vec<(hir::BodyId, hir::Body<'h>)>,
    pub bindings: HashMap<HirId, HirId>,
    pub nodes: hir::OwnerNodes<'h>,
    pub attributes: Vec<hir::DeferredDefinitionAttribute<'h>>,
    pub staged_parameters: Vec<(HirId, DefId)>,
    pub resolution: Vec<(HirId, hir::definitions::ResolutionEvidence)>,
}

#[cfg(test)]
pub(in crate::program) fn clone_member<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    member: OwnerId,
    owner: OwnerId,
    definitions: &HashMap<DefId, DefId>,
    fuel: &mut usize,
) -> Result<ClonedMember<'h>, Error> {
    clone_member_with_inputs(
        source,
        arena,
        member,
        owner,
        definitions,
        &HashMap::new(),
        fuel,
    )
}

pub(in crate::program) fn clone_member_with_inputs<'h>(
    source: &Package<'h>,
    arena: &'h HirArena,
    member: OwnerId,
    owner: OwnerId,
    definitions: &HashMap<DefId, DefId>,
    captures: &HashMap<HirId, InputCapture>,
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
    let (parameters, clause_params, clause_constraints) = match &item.kind {
        ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => (
            signature.params,
            signature.clause_params,
            signature.clause_constraints,
        ),
        ItemKind::Associated(definition) => (
            definition.parameters,
            definition.clause_params,
            definition.clause_constraints,
        ),
        _ => {
            return Err(error(
                item.span,
                "generated member draft requires a function or associated member",
            ));
        }
    };
    *fuel = fuel
        .checked_sub(definitions.len().saturating_add(captures.len()))
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
        captures: captures.clone(),
        capture_uses: BTreeSet::new(),
        self_type: None,
    };
    let result = (|| {
        rebind.charge(item.span, 0)?;
        let clause_params = rebind.clauses(clause_params)?;
        let mut cloned_parameters = Vec::new();
        let mut staged_parameters = Vec::new();
        for parameter in parameters {
            rebind.charge(parameter.span, 0)?;
            if parameter.ty.is_comptime() {
                let old = source
                    .staged_parameter_definitions
                    .get(&parameter.hir_id)
                    .ok_or_else(|| {
                        error(
                            parameter.span,
                            "generated staged parameter has no source identity",
                        )
                    })?;
                let new = definitions
                    .get(old)
                    .copied()
                    .filter(|new| new != old && new.is_valid())
                    .ok_or_else(|| {
                        error(
                            parameter.span,
                            "generated staged parameter requires a fresh definition identity",
                        )
                    })?;
                staged_parameters.push((rebind.id(parameter.hir_id), new));
            }
            cloned_parameters.push(rebind.parameter(parameter, 0)?);
        }
        let parameters = arena.alloc_fn_param_slice(cloned_parameters);
        let clause_constraints = rebind.constraints(clause_constraints)?;
        let mut item = item.clone();
        item.owner_id = owner;
        item.kind = match &item.kind {
            ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => {
                let mut signature = signature.clone();
                signature.params = parameters;
                signature.clause_params = clause_params;
                signature.clause_constraints = clause_constraints;
                signature.return_ty = signature
                    .return_ty
                    .map(|value| rebind.expr(value, 0))
                    .transpose()?;
                signature.return_binding = signature.return_binding.map(|id| rebind.id(id));
                match &item.kind {
                    ItemKind::Fn(_, body) => {
                        ItemKind::Fn(signature, rebind.body(*body, item.span, 0)?)
                    }
                    _ => ItemKind::FnDeclaration(signature),
                }
            }
            ItemKind::Associated(definition) => ItemKind::Associated(hir::item::AssociatedDecl {
                parameters,
                clause_params,
                clause_constraints,
                ty: rebind.expr(definition.ty, 0)?,
                default: definition
                    .default
                    .map(|value| rebind.expr(value, 0))
                    .transpose()?,
            }),
            _ => unreachable!(),
        };
        let attributes = rebind.attributes(member)?;
        let resolution = rebind.resolution()?;
        let mut nodes = hir::OwnerNodes::new();
        for parameter in parameters {
            let parameter = arena.alloc_param(hir::body::BodyParam::from_fn_param(parameter));
            nodes.insert(
                parameter.hir_id.local_id,
                hir::ItemLocalId::new(0),
                hir::Node::Param(parameter),
            );
        }
        for parameter in clause_params {
            nodes.insert(
                parameter.hir_id.local_id,
                hir::ItemLocalId::new(0),
                hir::Node::ClauseParam(parameter),
            );
        }
        Ok((item, resolution, attributes, nodes, staged_parameters))
    })();
    *fuel = rebind.budget;
    let (item, resolution, attributes, nodes, staged_parameters) = result?;
    Ok(ClonedMember {
        item,
        bodies: rebind.bodies,
        capture_uses: rebind.capture_uses,
        bindings: rebind.bindings,
        resolution,
        attributes,
        nodes,
        staged_parameters,
    })
}

#[cfg(test)]
mod tests;
