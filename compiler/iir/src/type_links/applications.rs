//! Recheck closed NF value domains against the retained declaration world.
use super::*;
use nir::normal_forms::{Head, Role};
use std::borrow::Cow;

impl TypeLinks<'_> {
    pub(super) fn validate_application(
        &self,
        head: &Head,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), String> {
        // Surrounding binders and runtime dependencies need their producer's
        // typed environment evidence. A closed value needs no such assumption.
        for argument in arguments {
            charge(argument, fuel)?;
            if argument.check_scope(&[]).is_err() {
                return Ok(());
            }
            let mut pending = vec![argument];
            while let Some(term) = pending.pop() {
                tick(fuel)?;
                if matches!(term, Term::Runtime(_) | Term::Rigid { .. }) {
                    return Ok(());
                }
                term.visit_children(|child| {
                    pending.push(child);
                    Ok::<_, String>(())
                })?;
            }
        }
        head.check_arguments_in_scope_with_callable_values_budgeted_in(
            arguments,
            |id| self.heads.get(&id).is_some_and(|h| h.role == Role::Tuple),
            |id| {
                self.heads
                    .get(&id)
                    .is_some_and(|h| h.role == Role::TypeList)
            },
            |ty, variant, fuel| {
                let definition = self.nominal(ty, fuel).ok()??;
                match (definition, variant) {
                    (NominalDefinition::Struct(s), None) => {
                        for (_, ty) in &s.fields {
                            charge(ty, fuel).ok()?;
                        }
                        Some(s.fields.iter().map(|(_, ty)| ty.clone()).collect())
                    }
                    (NominalDefinition::Enum(e), Some(index)) => {
                        let fields = &e.variants.get(index as usize)?.fields;
                        for ty in fields {
                            charge(ty, fuel).ok()?;
                        }
                        Some(fields.clone())
                    }
                    (NominalDefinition::Newtype(n), None) => {
                        charge(&n.representation, fuel).ok()?;
                        Some(vec![n.representation.clone()])
                    }
                    _ => None,
                }
            },
            |_, _| false,
            |value, _, fuel| {
                tick(fuel).ok()?;
                let signature = match value {
                    nir::CallableValueRef::Function {
                        declaration,
                        arguments,
                    } => {
                        for argument in arguments {
                            charge(argument, fuel).ok()?;
                        }
                        self.callables.get(&(declaration, arguments))?
                    }
                    nir::CallableValueRef::Method(method) => {
                        for argument in method.terms() {
                            charge(argument, fuel).ok()?;
                        }
                        self.signatures.get(&(
                            method.interface.declaration,
                            method.interface.arguments.as_slice(),
                            &method.subject,
                            method.member,
                            method.method_arguments.as_slice(),
                        ))?
                    }
                };
                Some(Cow::Borrowed(*signature))
            },
            &[],
            fuel,
            self.target,
        )
        .map_err(|error| format!("IIR normal-form application: {error}"))
    }
}
