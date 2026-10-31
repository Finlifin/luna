//! Shared owner-local parameter registration for generated callable telescopes.
use super::{DefinitionNodes, Error, charge, error};
use crate::common::FnParam;
use crate::*;

pub(super) fn register_parameters<'hir>(
    arena: &'hir HirArena,
    nodes: &mut DefinitionNodes<'hir>,
    parameters: &[FnParam<'hir>],
    fuel: &mut usize,
    diagnostic: &'static str,
) -> Result<&'hir [BodyParam<'hir>], Error> {
    let owner = nodes.root().owner;
    let work = parameters
        .len()
        .checked_mul(2)
        .ok_or_else(|| error("parameter assembly budget overflow", owner))?;
    if *fuel < work {
        return Err(Error {
            kind: "budget exhausted",
            owner: None,
        });
    }
    let mut params = Vec::with_capacity(parameters.len());
    for (index, param) in parameters.iter().enumerate() {
        charge(fuel)?;
        if nodes.reserved_parent(param.hir_id) != Some(nodes.root())
            || param.ty.hir_id != param.hir_id
            || param.index as usize != index
        {
            return Err(error(diagnostic, owner));
        }
        params.push(BodyParam::from_fn_param(param));
    }
    let params = arena.alloc_param_slice(params);
    for param in params {
        nodes.register(Node::Param(param), fuel)?;
    }
    Ok(params)
}
