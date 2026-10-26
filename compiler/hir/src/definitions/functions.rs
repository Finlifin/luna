//! Build a function descriptor and body from one owner-local node draft.
use super::{DefinitionNodes, Error, charge, error};
use crate::*;
use rustc_span::Span;
use std::fmt;

/// An unpublished function and its separately stored body. Both must enter the
/// same DefinitionBatch and pass ordinary semantic publication before use.
#[derive(Debug)]
pub struct AssembledFunction<'hir> {
    pub definition: OwnerInfo<'hir>,
    pub body_id: BodyId,
    pub body: Body<'hir>,
}

impl fmt::Display for AssembledFunction<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(assembled-function (definition {}) (body-id {}) (body {}))",
            self.definition.node.expect_item(),
            self.body_id,
            self.body
        )
    }
}

/// The callback constructs a complete ordinary HIR signature and body, retaining
/// parameter dependencies, clauses, defaults and phase/unsafe modifiers. It
/// reserves parameter identities and the body expression directly under root,
/// but leaves their registration to this function. It reserves and registers all
/// other nodes, including signature types and body children, using the same fuel.
///
/// Body parameters are derived from the signature, never supplied separately.
/// Identity, index, parent and unfinished-reservation errors return no result.
/// This does not type-check, publish or execute generated code; callback side
/// effects and arena allocation are not rolled back on failure. Effect and
/// declaration-only items continue through their own ordinary HIR item kinds.
pub fn assemble_function<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make_function: impl FnOnce(
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<(FnSig<'hir>, &'hir Expr<'hir>), Error>,
) -> Result<AssembledFunction<'hir>, Error> {
    let (definition, body) = assemble_callable(arena, owner, name, span, fuel, |nodes, fuel| {
        let (signature, body) = make_function(nodes, fuel)?;
        Ok((signature, Some(body)))
    })?;
    let (body_id, body) = body.expect("function callback always supplies a body");
    Ok(AssembledFunction {
        definition,
        body_id,
        body,
    })
}

/// Assemble a bodyless function contract, for example a required trait member.
/// Parameters are reserved by the callback and registered by this entrypoint;
/// other signature nodes follow the same callback contract as assemble_function.
/// This creates no body-table entry. Whether a bodyless declaration is legal in
/// its containing definition remains a checked-publication decision.
pub fn assemble_function_declaration<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make_signature: impl FnOnce(&mut DefinitionNodes<'hir>, &mut usize) -> Result<FnSig<'hir>, Error>,
) -> Result<OwnerInfo<'hir>, Error> {
    let (definition, _) = assemble_callable(arena, owner, name, span, fuel, |nodes, fuel| {
        Ok((make_signature(nodes, fuel)?, None))
    })?;
    Ok(definition)
}

fn assemble_callable<'hir>(
    arena: &'hir HirArena,
    owner: OwnerId,
    name: Ident,
    span: Span,
    fuel: &mut usize,
    make: impl FnOnce(
        &mut DefinitionNodes<'hir>,
        &mut usize,
    ) -> Result<(FnSig<'hir>, Option<&'hir Expr<'hir>>), Error>,
) -> Result<(OwnerInfo<'hir>, Option<(BodyId, Body<'hir>)>), Error> {
    let mut nodes = DefinitionNodes::new(owner)?;
    charge(fuel)?;
    let (signature, value) = make(&mut nodes, fuel)?;
    let work = signature
        .params
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(1 + usize::from(value.is_some())))
        .ok_or_else(|| error("function assembly budget overflow", owner))?;
    if *fuel < work {
        return Err(Error {
            kind: "budget exhausted",
            owner: None,
        });
    }
    if value.is_some_and(|value| nodes.reserved_parent(value.hir_id) != Some(nodes.root())) {
        return Err(error(
            "function body root must be reserved under its owner",
            owner,
        ));
    }
    let mut params = Vec::with_capacity(signature.params.len());
    for (index, param) in signature.params.iter().enumerate() {
        charge(fuel)?;
        if nodes.reserved_parent(param.hir_id) != Some(nodes.root())
            || param.ty.hir_id != param.hir_id
            || param.index as usize != index
        {
            return Err(error(
                "function parameter identity or index is inconsistent",
                owner,
            ));
        }
        params.push(BodyParam::from_fn_param(param));
    }
    let params = arena.alloc_param_slice(params);
    for param in params {
        nodes.register(Node::Param(param), fuel)?;
    }
    let body = if let Some(value) = value {
        nodes.register(Node::Expr(value), fuel)?;
        Some((BodyId::new(value.hir_id), Body { params, value }))
    } else {
        None
    };
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: name,
        kind: match &body {
            Some((body_id, _)) => ItemKind::Fn(signature, *body_id),
            None => ItemKind::FnDeclaration(signature),
        },
        span,
    });
    Ok((nodes.finish(item, fuel)?, body))
}
