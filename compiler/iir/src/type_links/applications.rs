//! Recheck closed NF value domains against the retained declaration world.
use super::*;
use nir::normal_forms::Head;
use nir::value_admission::{DomainQueries, Error};
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
        let nominal_fields = |ty: &Term, variant: Option<u32>, fuel: &mut usize| {
            let definition = self
                .nominal(ty, fuel)?
                .ok_or_else(|| Error::from("nominal representation evidence unavailable"))?;
            match (definition, variant) {
                (NominalDefinition::Struct(s), None) => {
                    for (_, ty) in &s.fields {
                        charge(ty, fuel)?;
                    }
                    Ok(s.fields.iter().map(|(_, ty)| ty.clone()).collect())
                }
                (NominalDefinition::Enum(e), Some(index)) => {
                    let fields = &e
                        .variants
                        .get(index as usize)
                        .ok_or_else(|| Error::from("enum variant index is unavailable"))?
                        .fields;
                    for ty in fields {
                        charge(ty, fuel)?;
                    }
                    Ok(fields.clone())
                }
                (NominalDefinition::Newtype(n), None) => {
                    charge(&n.representation, fuel)?;
                    Ok(vec![n.representation.clone()])
                }
                _ => Err("nominal variant kind mismatch".into()),
            }
        };
        let callable = |value: nir::CallableValueRef<'_>, fuel: &mut usize| {
            tick(fuel)?;
            let signature = match value {
                nir::CallableValueRef::Function {
                    declaration,
                    arguments,
                } => {
                    for argument in arguments {
                        charge(argument, fuel)?;
                    }
                    self.callables.get(&(declaration, arguments))
                }
                nir::CallableValueRef::Method(method) => {
                    for argument in method.terms() {
                        charge(argument, fuel)?;
                    }
                    self.signatures.get(&(
                        method.interface.declaration,
                        method.interface.arguments.as_slice(),
                        &method.subject,
                        method.member,
                        method.method_arguments.as_slice(),
                    ))
                }
            };
            Ok(signature.map(|signature| Cow::Borrowed(*signature)))
        };
        head.check_arguments(
            arguments,
            &[],
            fuel,
            &mut DomainQueries {
                layout: &mut |ty, variant, fuel| {
                    if variant.is_none() {
                        if let Some(fields) = nir::normal_forms::registered_tuple_fields(ty, |id| {
                            self.heads.get(&id).copied()
                        }) {
                            nir::type_computations::charge_term(ty, fuel)?;
                            return Ok(fields.to_vec());
                        }
                    }
                    nominal_fields(ty, variant, fuel)
                },
                declared: &mut |_, _, _| false,
                collection: &mut |domain, fuel| {
                    let Term::NormalForm { head, .. } = domain else {
                        return Ok(None);
                    };
                    let Some(element) = self.heads.get(head).and_then(|h| h.list_element(domain))
                    else {
                        return Ok(None);
                    };
                    nir::type_computations::charge_term(element, fuel)?;
                    Ok(Some(element.clone()))
                },
                callable: &mut |value, _, fuel| callable(value, fuel),
                target: self.target,
            },
        )
        .map_err(|error| format!("IIR normal-form application: {error}"))
    }
}
