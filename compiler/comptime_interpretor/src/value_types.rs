//! Shared value shape checking with explicit formation and callable obligations.
//! Passive observations require retained evidence. Active boundaries discharge
//! their private pending obligations in the receiving world before accepting.
use super::*;

#[cfg(test)]
mod tests;

enum ValueObligation {
    Nominal(Term),
    RuntimeMethod(TraitMethodIdentity),
    CallableType { signature: Term, expected: Term },
}

struct ValueChecks {
    // None denotes a read-only probe requiring retained evidence.
    obligations: Option<Vec<ValueObligation>>,
    remaining: usize,
    failure: Option<ErrorKind>,
}
impl ValueChecks {
    fn admit<T>(
        &mut self,
        operation: impl FnOnce(&mut usize) -> Result<T, nir::NominalAdmissionError>,
    ) -> Option<T> {
        self.observe(|fuel| {
            operation(fuel).map_err(|error| match error {
                nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
                nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
            })
        })
    }
    fn defers_contracts(&self) -> bool {
        self.obligations.is_some()
    }
    fn observe<T>(
        &mut self,
        operation: impl FnOnce(&mut usize) -> Result<T, ErrorKind>,
    ) -> Option<T> {
        if self.failure.is_some() {
            return None;
        }
        match operation(&mut self.remaining) {
            Ok(value) => Some(value),
            Err(failure) => {
                self.failure = Some(failure);
                None
            }
        }
    }
    fn visit(&mut self) -> bool {
        // Retained probes previously had no per-value traversal allowance.
        // Their local fuel bounds observations, not a new scalar width limit.
        if self.obligations.is_none() {
            return self.failure.is_none();
        }
        self.observe(|fuel| {
            *fuel = fuel.checked_sub(1).ok_or(ErrorKind::StepsExhausted)?;
            Ok(())
        })
        .is_some()
    }
    fn record_nominal(&mut self, ty: &Term) -> bool {
        if self.obligations.is_none() {
            return true;
        }
        if self
            .observe(|fuel| {
                nir::type_computations::charge_term(ty, fuel).map_err(|_| ErrorKind::StepsExhausted)
            })
            .is_none()
        {
            return false;
        }
        self.obligations
            .as_mut()
            .unwrap()
            .push(ValueObligation::Nominal(ty.clone()));
        true
    }
    fn record_method(&mut self, method: &TraitMethodIdentity) -> bool {
        if !self.defers_contracts() {
            return true;
        }
        if self
            .observe(|fuel| {
                for term in method.terms() {
                    nir::type_computations::charge_term(term, fuel)
                        .map_err(|_| ErrorKind::StepsExhausted)?;
                }
                Ok(())
            })
            .is_none()
        {
            return false;
        }
        self.obligations
            .as_mut()
            .unwrap()
            .push(ValueObligation::RuntimeMethod(method.clone()));
        true
    }

    fn callable_matches(
        &mut self,
        signature: &Term,
        expected: &Term,
        values: &associated_catalog::Catalog<'_>,
    ) -> bool {
        if self.failure.is_some() {
            return false;
        }
        // Retained evidence settles ordinary signatures without allocating a
        // pending task. Only a missing associated view needs active observation;
        // budget failures and known mismatches must not trigger a second query.
        match selected_types::same(signature, expected, values, &mut self.remaining) {
            Ok(equal) => return equal,
            Err(ErrorKind::UnmaterializedDispatch) if self.defers_contracts() => {}
            Err(failure) => {
                self.failure = Some(failure);
                return false;
            }
        }
        if self
            .observe(|fuel| {
                for term in [signature, expected] {
                    nir::type_computations::charge_term(term, fuel)
                        .map_err(|_| ErrorKind::StepsExhausted)?;
                }
                Ok(())
            })
            .is_none()
        {
            return false;
        }
        self.obligations
            .as_mut()
            .unwrap()
            .push(ValueObligation::CallableType {
                signature: signature.clone(),
                expected: expected.clone(),
            });
        true
    }

    fn same(
        &mut self,
        left: &Term,
        right: &Term,
        values: &associated_catalog::Catalog<'_>,
    ) -> bool {
        self.observe(|fuel| selected_types::same(left, right, values, fuel)) == Some(true)
    }
}

impl Interpreter<'_> {
    fn shape_nominal_arguments(
        &self,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        checks: &mut ValueChecks,
    ) -> bool {
        checks
            .admit(|fuel| {
                nir::validate_declaration_arguments_with_collections_in(
                    parameters,
                    domains,
                    arguments,
                    fuel,
                    |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| {
                        matches!(
                            value,
                            Term::Rigid { declaration, sort: nir::Sort::ComptimeValue }
                                if self.abstract_value_parameters.get(declaration) == Some(domain)
                        )
                    },
                    |domain, fuel| self.admitted_collection_domain(domain, fuel),
                    self.target,
                )
            })
            .is_some()
    }

    fn shape_nominal_capabilities(
        &self,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        capabilities: &[nir::program::TraitRequirement],
        checks: &mut ValueChecks,
    ) -> bool {
        // Argument admission has already checked this telescope. Preparing its
        // binder directory still consumes work even when obligations are deferred.
        let Some(binders) = checks.observe(|fuel| {
            *fuel = fuel
                .checked_sub(parameters.len())
                .ok_or(ErrorKind::StepsExhausted)?;
            for domain in domains {
                nir::type_computations::charge_term(domain, fuel)
                    .map_err(|_| ErrorKind::StepsExhausted)?;
            }
            nir::declaration_parameters(parameters, domains).map_err(|_| ErrorKind::Type)
        }) else {
            return false;
        };
        checks.defers_contracts()
            || checks.observe(|fuel| {
                self.capabilities_hold_parameters(capabilities, &binders, arguments, fuel)
            }) == Some(true)
    }

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
        let mut checks = ValueChecks {
            obligations: Some(vec![]),
            remaining: before,
            failure: None,
        };
        let matched = self.has_type_at(value, ty, 0, &mut checks);
        let ValueChecks {
            obligations,
            remaining,
            failure,
        } = checks;
        self.budget.steps -= (before - remaining) as u64;
        if let Some(kind) = failure {
            return Err(self.failure(kind, source));
        }
        if !matched {
            return Err(self.failure(ErrorKind::Type, source));
        }
        for obligation in obligations.expect("active shape obligations") {
            match obligation {
                ValueObligation::Nominal(domain) => self.admit_type(&domain, 0)?,
                ValueObligation::RuntimeMethod(method) => {
                    self.admit_runtime_method_identity(&method)?;
                }
                ValueObligation::CallableType {
                    signature,
                    expected,
                } => {
                    if !self.observe_same_type(
                        &signature,
                        &expected,
                        source.unwrap_or(SourceRange { start: 0, end: 0 }),
                    )? {
                        return Err(self.failure(ErrorKind::Type, source));
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn has_type(&self, value: &Value, ty: &Term) -> bool {
        // Legacy read-only probes have no mutable invocation owner. Use one
        // finite allowance for the entire probe, never one per projection.
        self.has_type_at(
            value,
            ty,
            0,
            &mut ValueChecks {
                obligations: None,
                remaining: 100_000,
                failure: None,
            },
        )
    }
    fn has_type_at(
        &self,
        value: &Value,
        ty: &Term,
        depth: usize,
        checks: &mut ValueChecks,
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
            checks.failure = Some(ErrorKind::DepthExhausted);
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
                checks
                    .admit(|fuel| {
                        d.validate_completed_arguments_with_collections_in(
                            arguments,
                            fuel,
                            |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                            |value, domain, _| {
                                matches!(
                                    value,
                                    Term::Rigid { declaration, sort: nir::Sort::ComptimeValue }
                                        if self.abstract_value_parameters.get(declaration) == Some(domain)
                                )
                            },
                            |domain, fuel| self.admitted_collection_domain(domain, fuel),
                            self.target,
                        )
                    })
                    .is_some()
                    && self.shape_nominal_capabilities(
                        &d.type_parameters,
                        &d.parameter_types,
                        arguments,
                        &d.capabilities,
                        checks,
                    )
                    && checks
                        .observe(|fuel| {
                            d.representation_budgeted(arguments, &[], fuel)
                                .map_err(|error| match error {
                                    nir::newtypes::ConversionError::Budget => {
                                        ErrorKind::StepsExhausted
                                    }
                                    _ => ErrorKind::Type,
                                })
                        })
                        .is_some_and(|representation| {
                            self.has_type_at(value, &representation, depth + 1, checks)
                        })
            });
        }

        if matches!(ty, Term::AssociatedType { .. }) {
            return checks
                .observe(|fuel| selected_types::view(ty, &self.associated_values, fuel))
                .is_some_and(|ty| self.has_type_at(value, &ty, depth + 1, checks));
        }
        if matches!(ty, Term::Nominal { .. }) && !self.associated_values.is_empty() {
            let Some(selected) =
                checks.observe(|fuel| selected_types::view(ty, &self.associated_values, fuel))
            else {
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
                checks.same(&pointer.pointee, inner, &self.associated_values)
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
                if declaration != expected || actual_arguments != arguments {
                    return false;
                }
                let Some(definition) = self.enums.get(declaration) else {
                    return false;
                };
                let Some(variant) = definition.variants.get(*index) else {
                    return false;
                };
                self.shape_nominal_arguments(
                    &definition.type_parameters,
                    &definition.parameter_types,
                    arguments,
                    checks,
                ) && self.shape_nominal_capabilities(
                    &definition.type_parameters,
                    &definition.parameter_types,
                    arguments,
                    &definition.capabilities,
                    checks,
                ) && checks
                    .admit(|fuel| {
                        nir::normal_forms::instantiate_value_fields_checked(
                            &definition.type_parameters,
                            &definition.parameter_types,
                            arguments,
                            variant.fields.iter(),
                            fuel,
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
                self.comptime_only
                    && object.entries().iter().all(|entry| {
                        let typed = entry.value();
                        checks
                            .observe(|fuel| self.inspect_type_description(&typed.ty, fuel))
                            .is_some()
                            && self.has_type_at(&typed.value, &typed.ty, depth + 1, checks)
                    })
            }
            (Value::List { ty: actual, values }, ty) => {
                if values.len() > 100_000 {
                    checks.failure = Some(ErrorKind::RepresentationSizeLimit);
                    return false;
                }
                self.comptime_only
                    && actual == ty
                    && checks
                        .admit(|fuel| self.admitted_collection_domain(ty, fuel))
                        .flatten()
                        .is_some_and(|element| {
                            // The summary stores only identical scalar discriminants.
                            // Resolve and check the current element domain normally;
                            // no equality, head formation or target decision is cached.
                            if values
                                .summary()
                                .is_some_and(|summary| summary.uniform_scalar.is_some())
                            {
                                values.first().is_some_and(|value| {
                                    self.has_type_at(value, &element, depth + 1, checks)
                                })
                            } else {
                                values.iter().all(|value| {
                                    self.has_type_at(value, &element, depth + 1, checks)
                                })
                            }
                        })
            }
            (Value::Description(_), Term::Primitive(Primitive::Type)) => self.comptime_only,
            (Value::TypeParameter(id), Term::Primitive(Primitive::Type)) => {
                self.abstract_type_parameters.contains(id)
            }
            (Value::Type(value), Term::Primitive(Primitive::Type)) => checks
                .observe(|fuel| self.inspect_type_description(value, fuel))
                .is_some(),
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
                self.structs.get(declaration).is_some_and(|definition| {
                    self.shape_nominal_arguments(
                        &definition.type_parameters,
                        &definition.parameter_types,
                        arguments,
                        checks,
                    ) && self.shape_nominal_capabilities(
                        &definition.type_parameters,
                        &definition.parameter_types,
                        arguments,
                        &definition.capabilities,
                        checks,
                    ) && checks
                        .admit(|fuel| {
                            nir::normal_forms::instantiate_value_fields_checked(
                                &definition.type_parameters,
                                &definition.parameter_types,
                                arguments,
                                definition.fields.iter().map(|(_, ty)| ty),
                                fuel,
                            )
                        })
                        .is_some_and(|types| {
                            fields.len() == types.len()
                                && fields.iter().zip(&types).all(|(value, ty)| {
                                    self.has_type_at(value, ty, depth + 1, checks)
                                })
                        })
                })
            }
            (Value::Function(id), ty @ Term::NormalFormFunction(..)) => checks
                .admit(|fuel| {
                    self.admitted_callable_signature(
                        nir::CallableValueRef::Function {
                            declaration: *id,
                            arguments: &[],
                        },
                        &[],
                        fuel,
                    )
                })
                .flatten()
                .is_some_and(|signature| {
                    checks.callable_matches(&signature, ty, &self.associated_values)
                }),
            (
                Value::Function(id),
                ty @ (Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)),
            ) => self.functions.interface(id).is_some_and(|f| {
                f.type_parameters.is_empty()
                    && f.phase != nir::program::FunctionPhase::NormalForm
                    && (self.comptime_only
                        || (f.requirements.is_empty() && f.capabilities.is_empty()))
                    && (f.phase == nir::program::FunctionPhase::Comptime)
                        == ty.is_comptime_callable()
                    && f.is_unsafe == ty.is_unsafe_callable()
                    && checks.callable_matches(
                        &f.signature,
                        ty.callable_signature(),
                        &self.associated_values,
                    )
            }),
            (
                Value::FunctionInstance {
                    declaration,
                    arguments,
                },
                ty,
            ) => checks
                .admit(|fuel| {
                    self.admitted_callable_signature(
                        nir::CallableValueRef::Function {
                            declaration: *declaration,
                            arguments,
                        },
                        &[],
                        fuel,
                    )
                })
                .flatten()
                .is_some_and(|signature| {
                    checks.callable_matches(&signature, ty, &self.associated_values)
                }),
            (Value::RuntimeTraitMethod(identity), ty) => {
                let TraitMethodIdentity {
                    interface,
                    subject,
                    member,
                    method_arguments: _,
                } = identity.as_ref();
                // An active boundary obtains method evidence in this world.
                // A cold dispatch table is not evidence against membership.
                (checks.defers_contracts()
                    || self.method_bodies.contains_key(&(interface.clone(), subject.clone(), *member))
                    || self.resolved_method_bodies.contains_key(&(interface.clone(), subject.clone(), *member)))
                    && self
                        .method_contracts
                        .get(&(interface.declaration, *member))
                        .is_some_and(|(_, contract)| {
                            !contract.signature.is_comptime_callable()
                                && (contract.program.preconditions.is_empty()
                                    || (self.comptime_only && matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))))
                                && checks.admit(|fuel| self.admitted_callable_signature(
                                    nir::CallableValueRef::Method(identity), &[], fuel,
                                )).flatten().is_some_and(|signature| {
                                    checks.record_method(identity)
                                        && checks.callable_matches(&signature, ty, &self.associated_values)
                                })
                        })
            }
            (Value::Closure(closure), ty) => {
                checks.callable_matches(&closure.ty, ty, &self.associated_values)
            }
            _ => false,
        }
    }
}
