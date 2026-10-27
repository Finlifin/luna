//! Declaration identities, distinct from references into another code context.
use super::{Error, charge, error};
use crate::common::FnParam;
use crate::*;
use std::collections::BTreeSet;

/// Only declarations introduce clause identities; template links borrow them.
pub fn clauses<'hir>(item: &Item<'hir>) -> &'hir [ClauseParam<'hir>] {
    match &item.kind {
        ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _) => {
            sig.clause_params
        }
        ItemKind::Struct(def) => def.clause_params,
        ItemKind::Enum(def) => def.clause_params,
        ItemKind::Associated(def) => def.clause_params,
        ItemKind::Impl(def) => def.clause_params,
        ItemKind::Trait(def) => def.clause_params,
        ItemKind::Deriver(def) => def.clause_params,
        ItemKind::Derive(def) => def.clause_params,
        ItemKind::EffectGroup(def) => def.clause_params,
        ItemKind::TypeAlias(_, params, _) | ItemKind::Newtype(_, params, _) => params,
        ItemKind::FunctionTemplateInstance { .. }
        | ItemKind::TestTemplateInstance { .. }
        | ItemKind::Mod(_)
        | ItemKind::Use(_)
        | ItemKind::Const(..)
        | ItemKind::Invalid => &[],
    }
}

pub(super) fn validate(item: &Item<'_>, fuel: &mut usize) -> Result<(), Error> {
    let mut members = Members {
        owner: item.owner_id,
        seen: BTreeSet::new(),
        fuel,
    };
    let (params, constraints) = match &item.kind {
        ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _) => {
            members.parameters(sig.params)?;
            if let Some(id) = sig.return_binding {
                members.id(id)?;
            }
            (sig.clause_params, sig.clause_constraints)
        }
        ItemKind::Struct(def) => {
            members.fields(def.fields)?;
            (def.clause_params, def.clause_constraints)
        }
        ItemKind::Enum(def) => {
            // Slice iterators bound auxiliary storage by nesting depth. Each
            // visited variant consumes fuel before descending into its payload.
            let mut pending = vec![def.variants.iter()];
            while let Some(iter) = pending.last_mut() {
                let Some(variant) = iter.next() else {
                    pending.pop();
                    continue;
                };
                members.id(variant.hir_id)?;
                match &variant.kind {
                    VariantKind::Struct(fields) => members.fields(fields)?,
                    VariantKind::SubEnum(variants) => pending.push(variants.iter()),
                    _ => {}
                }
            }
            (def.clause_params, def.clause_constraints)
        }
        ItemKind::Associated(def) => {
            members.parameters(def.parameters)?;
            (def.clause_params, def.clause_constraints)
        }
        ItemKind::Impl(def) => (def.clause_params, def.clause_constraints),
        ItemKind::Trait(def) => (def.clause_params, def.clause_constraints),
        ItemKind::Derive(def) => (def.clause_params, def.clause_constraints),
        ItemKind::Deriver(def) => {
            members.parameters(def.parameters)?;
            (def.clause_params, def.clause_constraints)
        }
        ItemKind::EffectGroup(def) => (def.clause_params, def.clause_constraints),
        ItemKind::TypeAlias(_, params, constraints) | ItemKind::Newtype(_, params, constraints) => {
            (*params, *constraints)
        }
        ItemKind::FunctionTemplateInstance {
            explicit_parameters,
            parameters,
            ..
        } => {
            // Linked signatures and formal IDs belong to the template declaration.
            if let Some(params) = explicit_parameters {
                members.parameters(params)?;
                if params.len() != parameters.len() {
                    return Err(error(
                        "template parameter bindings differ from explicit parameters",
                        item.owner_id,
                    ));
                }
                for (param, input) in params.iter().zip(parameters) {
                    charge(members.fuel)?;
                    if param.hir_id != input.binding {
                        return Err(error(
                            "template parameter bindings differ from explicit parameters",
                            item.owner_id,
                        ));
                    }
                }
            } else {
                for input in parameters {
                    members.id(input.binding)?;
                }
            }
            return Ok(());
        }
        ItemKind::TestTemplateInstance { ports, .. } => {
            // The linked interface is likewise not instance-owned.
            for port in ports {
                members.id(port.binding)?;
            }
            return Ok(());
        }
        ItemKind::Mod(_) | ItemKind::Use(_) | ItemKind::Const(..) | ItemKind::Invalid => {
            return Ok(());
        }
    };
    // Resolved clause references use DefId rather than HirId. Distinct nodes
    // must not alias the same substitution slot within a signature.
    let mut clause_definitions = BTreeSet::new();
    for param in params {
        members.id(param.hir_id)?;
        if let Some(definition) = param.definition {
            if !definition.is_valid() || !clause_definitions.insert(definition) {
                return Err(error(
                    "invalid or duplicate clause definition identity",
                    item.owner_id,
                ));
            }
        }
    }
    for constraint in constraints {
        members.id(constraint.hir_id)?;
    }
    Ok(())
}

struct Members<'a> {
    owner: OwnerId,
    seen: BTreeSet<ItemLocalId>,
    fuel: &'a mut usize,
}
impl Members<'_> {
    fn id(&mut self, id: HirId) -> Result<(), Error> {
        charge(self.fuel)?;
        if id.owner != self.owner || id.local_id.raw() == 0 || id.local_id == ItemLocalId::INVALID {
            return Err(error(
                "member identity must be non-root and declaration-owned",
                self.owner,
            ));
        }
        if !self.seen.insert(id.local_id) {
            return Err(error("duplicate declaration member identity", self.owner));
        }
        Ok(())
    }
    fn fields(&mut self, fields: &[FieldDef<'_>]) -> Result<(), Error> {
        for field in fields {
            self.id(field.hir_id)?;
        }
        Ok(())
    }
    fn parameters(&mut self, params: &[FnParam<'_>]) -> Result<(), Error> {
        for param in params {
            self.id(param.hir_id)?;
            if param.ty.hir_id != param.hir_id {
                return Err(error(
                    "parameter and type parameter identities differ",
                    self.owner,
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{TyParam, TyParamKind};
    use crate::item::{FnModifiers, FunctionTemplateInput};
    use rustc_span::DUMMY_SP;

    fn id(local: u32) -> HirId {
        HirId::new(OwnerId::new(LocalDefId::new(1)), ItemLocalId::new(local))
    }
    fn name() -> Ident {
        Ident::new(Symbol::intern("p"), DUMMY_SP)
    }
    fn item(kind: ItemKind<'_>) -> Item<'_> {
        Item {
            owner_id: id(0).owner,
            ident: name(),
            kind,
            span: DUMMY_SP,
        }
    }
    fn parameter() -> FnParam<'static> {
        FnParam {
            hir_id: id(1),
            name: name(),
            index: 0,
            ty: TyParam {
                hir_id: id(1),
                kind: TyParamKind::Itself { is_ref: false },
                flags: 0,
                span: DUMMY_SP,
            },
            span: DUMMY_SP,
        }
    }
    #[test]
    fn parameter_mirrors_and_return_binding_are_checked() {
        let param = parameter();
        let signature = |params| FnSig {
            params,
            return_ty: None,
            return_bind: Some(name()),
            return_binding: Some(id(2)),
            modifiers: FnModifiers::default(),
            clause_params: &[],
            clause_constraints: &[],
            span: DUMMY_SP,
        };
        assert!(
            validate(
                &item(ItemKind::FnDeclaration(signature(std::slice::from_ref(
                    &param
                )))),
                &mut 20
            )
            .is_ok()
        );
        let mut param = parameter();
        param.ty.hir_id = id(3);
        assert!(
            validate(
                &item(ItemKind::FnDeclaration(signature(std::slice::from_ref(
                    &param
                )))),
                &mut 20
            )
            .is_err()
        );
        let param = parameter();
        let mut sig = signature(std::slice::from_ref(&param));
        sig.return_binding = Some(id(1));
        assert!(validate(&item(ItemKind::FnDeclaration(sig)), &mut 20).is_err());
    }
    #[test]
    fn explicit_template_parameters_mirror_bindings_and_allow_foreign_formals() {
        let param = parameter();
        let foreign = HirId::new(OwnerId::new(LocalDefId::new(99)), ItemLocalId::new(1));
        let template = Expr {
            hir_id: id(3),
            kind: ExprKind::TyVoid,
            span: DUMMY_SP,
        };
        let make = |binding| {
            item(ItemKind::FunctionTemplateInstance {
                template: &template,
                signature: None,
                explicit_parameters: Some(std::slice::from_ref(&param)),
                parameters: vec![FunctionTemplateInput {
                    name: name(),
                    binding,
                    formal: Some(foreign),
                }],
                modifiers: FnModifiers::default(),
                body: BodyId { hir_id: id(4) },
            })
        };
        assert!(validate(&make(id(1)), &mut 20).is_ok());
        assert!(validate(&make(id(2)), &mut 20).is_err());
        assert!(validate(&make(foreign), &mut 20).is_err());
    }
    #[test]
    fn nested_variants_share_identity_namespace_and_budget() {
        let leaf = Variant {
            hir_id: id(2),
            ident: name(),
            kind: VariantKind::Unit,
            span: DUMMY_SP,
        };
        let variants = [Variant {
            hir_id: id(1),
            ident: name(),
            kind: VariantKind::SubEnum(std::slice::from_ref(&leaf)),
            span: DUMMY_SP,
        }];
        let make = |variants| {
            item(ItemKind::Enum(EnumDef {
                is_intrinsic: false,
                variants,
                configurations: vec![],
                clause_params: &[],
                clause_constraints: &[],
                nested_items: vec![],
            }))
        };
        assert_eq!(
            validate(&make(&variants), &mut 1).unwrap_err().kind,
            "budget exhausted"
        );
        assert!(validate(&make(&variants), &mut 2).is_ok());
        let duplicate = [variants[0].clone(), leaf.clone()];
        assert_eq!(
            validate(&make(&duplicate), &mut 10).unwrap_err().kind,
            "duplicate declaration member identity"
        );
    }

    #[test]
    fn test_template_interface_is_external_but_ports_are_owned() {
        use crate::item::{TestTemplateInterface, TestTemplatePort};
        let foreign = HirId::new(OwnerId::new(LocalDefId::new(99)), ItemLocalId::new(1));
        let expr = Expr {
            hir_id: foreign,
            kind: ExprKind::TyVoid,
            span: DUMMY_SP,
        };
        let make = |binding| {
            item(ItemKind::TestTemplateInstance {
                template: &expr,
                interface: Some(TestTemplateInterface {
                    parameter: foreign,
                    ty: &expr,
                }),
                ports: vec![TestTemplatePort {
                    name: name(),
                    binding,
                    formal: Some(foreign),
                }],
                body: BodyId { hir_id: id(3) },
            })
        };
        assert!(validate(&make(id(1)), &mut 10).is_ok());
        assert!(validate(&make(foreign), &mut 10).is_err());
        assert!(validate(&make(id(0)), &mut 10).is_err());
        assert!(
            validate(
                &make(HirId::new(id(0).owner, ItemLocalId::INVALID)),
                &mut 10
            )
            .is_err()
        );
    }

    #[test]
    fn clauses_cannot_reuse_field_identities() {
        let expr = Expr {
            hir_id: id(3),
            kind: ExprKind::TyVoid,
            span: DUMMY_SP,
        };
        let fields = [FieldDef {
            hir_id: id(1),
            ident: name(),
            ty: &expr,
            default: None,
            span: DUMMY_SP,
        }];
        let clause = |local| ClauseParam {
            hir_id: id(local),
            definition: None,
            name: name(),
            kind: ClauseParamKind::Type(name()),
            span: DUMMY_SP,
        };
        let make = |clause_params| {
            item(ItemKind::Struct(StructDef {
                fields: &fields,
                clause_params,
                clause_constraints: &[],
                nested_items: vec![],
            }))
        };
        let valid = [clause(2)];
        let duplicate = [clause(1)];
        assert!(validate(&make(&valid), &mut 10).is_ok());
        assert_eq!(
            validate(&make(&duplicate), &mut 10).unwrap_err().kind,
            "duplicate declaration member identity"
        );
    }
}
