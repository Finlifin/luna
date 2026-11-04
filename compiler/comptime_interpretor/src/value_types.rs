//! Shared value shape checking with explicit nominal formation obligations.
//! Passive observations require retained evidence. Active boundaries discharge
//! their private pending obligations in the receiving world before accepting.
use super::*;

#[cfg(test)]
mod tests;

enum NominalChecks {
    Retained,
    Pending {
        domains: Vec<Term>,
        remaining: usize,
        exhausted: bool,
    },
}
impl NominalChecks {
    fn defers_contracts(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }
    fn visit(&mut self) -> bool {
        let Self::Pending {
            remaining,
            exhausted,
            ..
        } = self
        else {
            return true;
        };
        if *remaining == 0 {
            *exhausted = true;
            return false;
        }
        *remaining -= 1;
        true
    }
    fn record_nominal(&mut self, ty: &Term) -> bool {
        let Self::Pending {
            domains,
            remaining,
            exhausted,
        } = self
        else {
            return true;
        };
        let charged = nir::type_computations::charge_term(ty, remaining);
        if charged.is_err() {
            *exhausted = true;
        } else {
            domains.push(ty.clone());
        }
        charged.is_ok()
    }
}

impl Interpreter<'_> {
    /// Check a value at an active boundary. Pending domains are local to this
    /// attempt: shape success alone cannot authorize the value or publish proof.
    pub(super) fn check_value_type(
        &mut self,
        value: &Value,
        ty: &Term,
        source: Option<SourceRange>,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return if self.has_type(value, ty) {
                Ok(())
            } else {
                Err(self.failure(ErrorKind::Type, source))
            };
        }
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut checks = NominalChecks::Pending {
            domains: vec![],
            remaining: before,
            exhausted: false,
        };
        let matched = self.has_type_at(value, ty, 0, &mut checks);
        let NominalChecks::Pending {
            domains,
            remaining,
            exhausted,
        } = checks
        else {
            unreachable!()
        };
        self.budget.steps -= (before - remaining) as u64;
        if exhausted {
            return Err(self.failure(ErrorKind::StepsExhausted, source));
        }
        if !matched {
            return Err(self.failure(ErrorKind::Type, source));
        }
        for domain in &domains {
            self.admit_type(domain, 0)?;
        }
        Ok(())
    }
    pub(super) fn has_type(&self, value: &Value, ty: &Term) -> bool {
        self.has_type_at(value, ty, 0, &mut NominalChecks::Retained)
    }
    fn has_type_at(
        &self,
        value: &Value,
        ty: &Term,
        depth: usize,
        checks: &mut NominalChecks,
    ) -> bool {
        if !checks.visit() {
            return false;
        }
        if self.graph.is_some() {
            match value {
                Value::RuntimeTraitMethod(identity) => {
                    return self
                        .iir_template_method_type(identity)
                        .is_some_and(|logical| logical == ty);
                }
                Value::FunctionInstance {
                    declaration,
                    arguments,
                } => {
                    return self
                        .iir_template_type(*declaration, arguments)
                        .is_some_and(|logical| logical == ty);
                }
                _ => {}
            }
        }
        if let Value::AbstractValue {
            declaration,
            ty: domain,
        } = value
        {
            return self.comptime_only
                && domain == ty
                && self.abstract_value_parameters.get(declaration) == Some(domain);
        }

        if depth >= 128 {
            return false;
        }
        if let Value::Optional { ty: actual, value } = value {
            let Term::Optional(payload) = ty else {
                return false;
            };
            return actual == ty
                && value
                    .as_ref()
                    .is_none_or(|value| self.has_type_at(value, payload, depth + 1, checks));
        }
        if let Value::ErrorResult {
            ty: actual,
            error,
            value,
        } = value
        {
            if actual != ty {
                return false;
            }
            let Term::ErrorResult { errors, result } = ty else {
                return false;
            };
            let payload = match error {
                None => result.as_ref(),
                Some(index) => {
                    let Some(label) = errors.get(*index) else {
                        return false;
                    };
                    label
                }
            };
            return self.has_type_at(value, payload, depth + 1, checks);
        }
        if let Value::Newtype { ty: actual, value } = value {
            if actual != ty {
                return false;
            }
            let Term::Nominal {
                declaration,
                arguments,
            } = ty
            else {
                return false;
            };
            if !checks.record_nominal(ty) {
                return false;
            }
            if self.graph.is_some() {
                return self.concrete_nominal(ty).is_some_and(|definition| {
                    if let ConcreteNominal::Newtype(definition) = definition {
                        self.has_type_at(value, &definition.representation, depth + 1, checks)
                    } else {
                        false
                    }
                });
            }
            return self.newtypes.get(declaration).is_some_and(|d| {
                d.validate_completed_arguments_with_collections_in(
                    arguments,
                    &mut 100_000,
                    |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), |domain, fuel| self.admitted_collection_domain(domain, fuel), self.target)
                .is_ok()
                    && d.parameter_binders().is_ok_and(|parameters| {
                        checks.defers_contracts() || self.capabilities_hold_parameters(&d.capabilities, &parameters, arguments)
                    })
                    && d.representation_budgeted(arguments, &[], &mut 100_000)
                        .is_ok_and(|representation| {
                            self.has_type_at(value, &representation, depth + 1, checks)
                        })
            });
        }
        if matches!(ty, Term::AssociatedType { .. }) {
            return selected_types::view(ty, &self.associated_values)
                .is_ok_and(|ty| self.has_type_at(value, &ty, depth + 1, checks));
        }
        if matches!(ty, Term::Nominal { .. }) && !self.associated_values.is_empty() {
            let Ok(selected) = selected_types::view(ty, &self.associated_values) else {
                return false;
            };
            if selected != *ty {
                return self.has_type_at(value, &selected, depth + 1, checks);
            }
        }
        // Collect the domain actually used by shape matching, after passive
        // projection selection. The unselected input is not a second domain.
        if matches!(ty, Term::Nominal { .. }) && !checks.record_nominal(ty) {
            return false;
        }
        if self.graph.is_some() {
            if let Term::Nominal {
                declaration,
                arguments,
            } = ty
            {
                let Some(definition) = self.concrete_nominal(ty) else {
                    return false;
                };
                return match (value, definition) {
                    (
                        Value::Record {
                            declaration: actual,
                            arguments: args,
                            fields,
                        },
                        ConcreteNominal::Struct(s),
                    ) => {
                        actual == declaration
                            && args == arguments
                            && fields.len() == s.fields.len()
                            && fields
                                .iter()
                                .zip(&s.fields)
                                .all(|(v, (_, t))| self.has_type_at(v, t, depth + 1, checks))
                    }
                    (
                        Value::Variant {
                            declaration: actual,
                            arguments: args,
                            index,
                            fields,
                        },
                        ConcreteNominal::Enum(e),
                    ) => {
                        actual == declaration
                            && args == arguments
                            && e.variants.get(*index).is_some_and(|variant| {
                                fields.len() == variant.fields.len()
                                    && fields
                                        .iter()
                                        .zip(&variant.fields)
                                        .all(|(v, t)| self.has_type_at(v, t, depth + 1, checks))
                            })
                    }
                    _ => false,
                };
            }
        }
        match (value, ty) {
            (Value::Pointer(pointer), Term::Pointer(inner)) => {
                selected_types::same(&pointer.pointee, inner, &self.associated_values)
            }
            (
                Value::Variant {
                    declaration,
                    arguments: actual_arguments,
                    index,
                    fields,
                },
                Term::Nominal {
                    declaration: expected,
                    arguments,
                },
            ) => {
                declaration == expected
                    && actual_arguments == arguments
                    && self
                        .enums
                        .get(declaration)
                        .filter(|definition| {
                            nir::validate_declaration_arguments_with_collections_in(
                                &definition.type_parameters,
                                &definition.parameter_types,
                                arguments,
                                &mut 100_000,
                                |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), |domain, fuel| self.admitted_collection_domain(domain, fuel), self.target)
                            .is_ok()
                                && definition.parameter_binders().is_ok_and(|parameters| {
                                    checks.defers_contracts() || self.capabilities_hold_parameters(
                                        &definition.capabilities,
                                        &parameters,
                                        arguments,
                                    )
                                })
                        })
                        .and_then(|definition| {
                            nir::normal_forms::instantiate_value_fields(
                                &definition.type_parameters,
                                &definition.parameter_types,
                                arguments,
                                definition.variants.get(*index)?.fields.iter(),
                                &mut 100_000,
                            )
                        })
                        .is_some_and(|types| {
                            fields.len() == types.len()
                                && fields
                                    .iter()
                                    .zip(&types)
                                    .all(|(value, ty)| self.has_type_at(value, ty, depth + 1, checks))
                        })
            }
            (Value::BitVec(value), Term::Primitive(Primitive::BitVec(width))) => {
                value.width() == *width
            }
            (Value::Object(object), Term::Primitive(Primitive::Object)) => {
                self.comptime_only && object.entries().iter().all(|entry| {
                    let typed = entry.value();
                    self.type_description_work(&typed.ty).is_some()
                        && self.has_type_at(&typed.value, &typed.ty, depth + 1, checks)
                })
            }
            (Value::List { ty: actual, values }, ty) => {
                self.comptime_only && actual == ty && values.len() <= 100_000
                    && self.list_element(ty).is_some_and(|element| {
                        // The summary stores only identical scalar discriminants.
                        // Resolve and check the current element domain normally;
                        // no equality, head formation or target decision is cached.
                        if values.summary().is_some_and(|summary| summary.uniform_scalar.is_some()) {
                            values.first().is_some_and(|value| self.has_type_at(value, element, depth + 1, checks))
                        } else {
                            values.iter().all(|value| self.has_type_at(value, element, depth + 1, checks))
                        }
                    })
            }
            (Value::TypeParameter(id), Term::Primitive(Primitive::Type)) => {
                self.abstract_type_parameters.contains(id)
            }
            (Value::Type(value), Term::Primitive(Primitive::Type)) => {
                self.type_description_work(value).is_some()
            }
            (Value::Float(_), Term::Primitive(Primitive::Float))
            | (Value::Integer(_), Term::Primitive(Primitive::Integer))
            | (Value::Float16(_), Term::Primitive(Primitive::F16))
            | (Value::BFloat16(_), Term::Primitive(Primitive::Bf16))
            | (Value::Float32(_), Term::Primitive(Primitive::F32))
            | (Value::Float64(_), Term::Primitive(Primitive::F64))
            | (Value::Unit, Term::Primitive(Primitive::Void))
            | (Value::Bool(_), Term::Primitive(Primitive::Bool))
            | (Value::Char(_), Term::Primitive(Primitive::Char))
            | (Value::String(_), Term::Primitive(Primitive::Str))
            | (Value::Symbol(_), Term::Primitive(Primitive::Symbol)) => true,
            (Value::SignedInteger(value), Term::Primitive(primitive)) => self
                .integer_layout(*primitive)
                .is_some_and(|(signed, width)| {
                    signed
                        && comptime_runtime::machine::MachineInteger::from_signed(
                            *value,
                            width as u16,
                        )
                        .is_ok()
                }),
            (Value::UnsignedInteger(value), Term::Primitive(primitive)) => self
                .integer_layout(*primitive)
                .is_some_and(|(signed, width)| {
                    !signed
                        && comptime_runtime::machine::MachineInteger::from_unsigned(
                            *value,
                            width as u16,
                        )
                        .is_ok()
                }),
            (Value::AnonymousRecord(values), Term::Record(types)) => {
                let mut names = std::collections::BTreeSet::new();
                values.len() == types.len()
                    && values
                        .iter()
                        .zip(types)
                        .all(|((name, value), (expected, ty))| {
                            name == expected
                                && names.insert(name)
                                && self.has_type_at(value, ty, depth + 1, checks)
                        })
            }
            (Value::Tuple(values), ty @ Term::NormalForm { head, .. }) => self
                .normal_forms
                .get(head)
                .and_then(|metadata| metadata.tuple_elements(ty))
                .is_some_and(|types| {
                    values.len() == types.len()
                        && values
                            .iter()
                            .zip(types)
                            .all(|(value, ty)| self.has_type_at(value, ty, depth + 1, checks))
                }),
            (Value::Tuple(values), Term::Tuple(types)) => {
                values.len() == types.len()
                    && values
                        .iter()
                        .zip(types)
                        .all(|(v, t)| self.has_type_at(v, t, depth + 1, checks))
            }
            (
                Value::Record {
                    declaration,
                    arguments: actual_arguments,
                    fields,
                },
                Term::Nominal {
                    declaration: expected,
                    arguments,
                },
            ) if declaration == expected && actual_arguments == arguments => {
                self.structs.get(declaration).is_some_and(|s| {
                    if nir::validate_declaration_arguments_with_collections_in(
                        &s.type_parameters,
                        &s.parameter_types,
                        arguments,
                        &mut 100_000,
                        |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), |domain, fuel| self.admitted_collection_domain(domain, fuel), self.target)
                    .is_err()
                        || !s.parameter_binders().is_ok_and(|parameters| {
                            checks.defers_contracts() || self.capabilities_hold_parameters(
                                &s.capabilities,
                                &parameters,
                                arguments,
                            )
                        })
                    {
                        return false;
                    }
                    nir::normal_forms::instantiate_value_fields(
                        &s.type_parameters,
                        &s.parameter_types,
                        arguments,
                        s.fields.iter().map(|(_, ty)| ty),
                        &mut 100_000,
                    )
                    .is_some_and(|types| {
                        types.len() == fields.len()
                            && types
                                .iter()
                                .zip(fields)
                                .all(|(ty, value)| self.has_type_at(value, ty, depth + 1, checks))
                    })
                })
            }
            (Value::Function(id), ty @ Term::NormalFormFunction(..)) => self.normal_forms.get(id)
                .is_some_and(|head| head.callable_type().ok().as_ref().is_some_and(|actual|
                    selected_types::same(actual, ty, &self.associated_values))),
            (Value::Function(id), ty @ (Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_))) => {
                self.functions.get(id).is_some_and(|f| {
                    f.type_parameters.is_empty()
                        && f.phase != nir::program::FunctionPhase::NormalForm
                        && (self.comptime_only
                            || (f.requirements.is_empty() && f.capabilities.is_empty()))
                        && (f.phase == nir::program::FunctionPhase::Comptime) == ty.is_comptime_callable()
                        && f.is_unsafe == ty.is_unsafe_callable()
                        && selected_types::same(
                            &f.signature,
                            ty.callable_signature(),
                            &self.associated_values,
                        )
                })
            }
            (
                Value::FunctionInstance {
                    declaration,
                    arguments,
                },
                ty,
            ) => self
                .normal_callable_signature_view(*declaration, arguments, &[], &mut 100_000)
                .is_some_and(|signature| {
                    selected_types::same(&signature, ty, &self.associated_values)
                }),
            (Value::RuntimeTraitMethod(identity), ty) => {
                let TraitMethodIdentity {
                    interface,
                    subject,
                    member,
                    method_arguments,
                } = identity.as_ref();
                (self.method_bodies.contains_key(&(interface.clone(), subject.clone(), *member))
                    || self.resolved_method_bodies.contains_key(&(interface.clone(), subject.clone(), *member)))
                    && self
                        .method_contracts
                        .get(&(interface.declaration, *member))
                        .is_some_and(|(parameters, contract)| {
                            let arguments: Vec<_> = std::iter::once(subject.clone())
                                .chain(interface.arguments.iter().cloned())
                                .chain(method_arguments.iter().cloned())
                                .collect();
                            let ids: Vec<_> = parameters
                                .iter()
                                .map(|parameter| parameter.declaration)
                                .collect();
                            self.validate_function_inputs(&ids, &arguments, &mut 100_000)
                                .is_ok()
                                && !contract.signature.is_comptime_callable()
                                && (contract.program.preconditions.is_empty()
                                    || (self.comptime_only && matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))))
                                && contract
                                    .signature
                                    .instantiate_parameters(parameters, &arguments)
                                    .is_ok_and(|signature| {
                                        signature.ordinary().is_ok_and(|signature| {
                                            selected_types::same(
                                                signature,
                                                ty,
                                                &self.associated_values,
                                            )
                                        })
                                    })
                        })
            }
            (Value::Closure(closure), ty) => {
                selected_types::same(&closure.ty, ty, &self.associated_values)
            }
            _ => false,
        }
    }
}
