//! Invocation-local borrowed indexes for completed type references.
use crate::{Package, Term};
use nir::DeclarationId;
use std::collections::HashSet;

pub(super) struct TypeLinks<'a> {
    callables: HashSet<(DeclarationId, &'a [Term])>,
    nominals: HashSet<&'a Term>,
    heads: HashSet<DeclarationId>,
}

fn tick(fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("IIR type link budget exhausted")?;
    Ok(())
}
fn charge(ty: &Term, fuel: &mut usize) -> Result<(), String> {
    nir::type_computations::charge_term(ty, fuel)
        .map_err(|e| format!("IIR type link key budget: {e}"))
}
impl<'a> TypeLinks<'a> {
    pub(super) fn new(package: &'a Package, fuel: &mut usize) -> Result<Self, String> {
        let mut result = Self {
            callables: HashSet::new(),
            nominals: HashSet::new(),
            heads: HashSet::new(),
        };
        for identity in &package.callable_identities {
            tick(fuel)?;
            for argument in &identity.key.arguments {
                charge(argument, fuel)?;
            }
            result
                .callables
                .insert((identity.key.declaration, &identity.key.arguments));
        }
        for nominal in &package.nominals {
            tick(fuel)?;
            charge(&nominal.ty, fuel)?;
            result.nominals.insert(&nominal.ty);
        }
        for head in &package.normal_forms {
            tick(fuel)?;
            result.heads.insert(head.declaration);
        }
        Ok(result)
    }

    pub(super) fn validate(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            tick(fuel)?;
            match ty {
                Term::FunctionValue {
                    declaration,
                    arguments,
                } => {
                    for argument in arguments {
                        charge(argument, fuel)?;
                    }
                    if !self
                        .callables
                        .contains(&(*declaration, arguments.as_slice()))
                    {
                        return Err("IIR frozen callable has no signature evidence".into());
                    }
                }
                Term::Nominal { .. } => {
                    charge(ty, fuel)?;
                    if !self.nominals.contains(ty) {
                        return Err("IIR nominal reference is dangling".into());
                    }
                }
                Term::NormalForm { head, .. } => {
                    if !self.heads.contains(head) {
                        return Err("IIR normal-form reference is dangling".into());
                    }
                }
                _ => {}
            }
            ty.visit_children(|child| {
                pending.push(child);
                Ok::<_, String>(())
            })?;
        }
        Ok(())
    }
}
