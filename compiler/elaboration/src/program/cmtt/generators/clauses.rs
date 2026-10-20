//! Clause defaults wait for completed Type evidence; probing never invokes them.
use super::*;

#[derive(Default)]
pub(in crate::program::cmtt) struct CompletedDefaults {
    pub(in crate::program::cmtt) types: BTreeMap<usize, Term>,
    pub(in crate::program::cmtt) clauses: BTreeMap<usize, Term>,
}

impl fmt::Display for CompletedDefaults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(completed-generator-defaults (types")?;
        for (slot, value) in &self.types {
            write!(f, " (slot {slot} {value})")?;
        }
        f.write_str(") (clauses")?;
        for (slot, value) in &self.clauses {
            write!(f, " (slot {slot} {value})")?;
        }
        f.write_str("))")
    }
}

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn generator_call_candidate(
        &mut self,
        owner: OwnerId,
        parameters: &[DeclarationId],
        plan: &mut [Term],
        pending: &[(usize, Vec<DeclarationId>)],
        deferred: &[usize],
        completed: &mut CompletedDefaults,
        base: &crate::calls::Scheme,
        evidence: &[Option<Term>],
        partial: &[crate::calls::InputContext],
        expected: Option<&Term>,
        span: Span,
    ) -> Result<crate::calls::CallCandidate, Error> {
        let domains = self.trait_domains(owner)?;
        let all = self.type_parameters[&owner].clone();
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "generator default inference budget exhausted"))?;
            let before = completed.types.len() + completed.clauses.len();
            let mut binders = Vec::new();
            let mut values = Vec::new();
            let mut unavailable = Vec::new();
            for slot in deferred {
                if domains[*slot] == p(P::Type) {
                    continue;
                }
                if let Some(value) = completed.clauses.get(slot) {
                    binders.push(nir::DeclarationParameter {
                        declaration: all[*slot],
                        sort: Sort::ComptimeValue,
                    });
                    values.push(self.copy_alias_type(value, span)?);
                } else {
                    unavailable.push(all[*slot]);
                }
            }
            let signature = self
                .copy_alias_type(&base.signature, span)?
                .instantiate_parameters(&binders, &values)
                .map_err(|e| error(span, e.to_string()))?;
            let Term::Function {
                parameters: inputs,
                result,
            } = signature.callable_signature()
            else {
                unreachable!()
            };
            // Other inputs can establish the Type needed by a default. Do not
            // reject an unresolved value index before that default can run.
            let evidence = evidence
                .iter()
                .zip(inputs)
                .map(|(value, input)| {
                    if unavailable.iter().any(|id| input.ty.mentions_rigid(*id)) {
                        None
                    } else {
                        value.clone()
                    }
                })
                .collect::<Vec<_>>();
            let expected =
                expected.filter(|_| !unavailable.iter().any(|id| result.mentions_rigid(*id)));
            let scheme = crate::calls::Scheme {
                parameter_sorts: base.parameter_sorts.clone(),
                type_parameters: base.type_parameters,
                signature,
            };
            let mut candidate = scheme
                .infer_partial_types_in_context(
                    &evidence,
                    expected,
                    &self.context,
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(span, e))?;
            let Term::Function {
                parameters: inputs, ..
            } = scheme.signature.callable_signature()
            else {
                unreachable!()
            };
            for (index, context) in partial.iter().enumerate() {
                if !unavailable
                    .iter()
                    .any(|id| inputs[index].ty.mentions_rigid(*id))
                {
                    candidate
                        .constrain_parameter_context(index, context, crate::ptc::Budget::default())
                        .map_err(|e| call_inference_error(span, e))?;
                }
            }
            self.settle_function_defaults(
                owner,
                parameters,
                plan,
                pending,
                &mut completed.types,
                &mut candidate,
                span,
            )?;
            for slot in deferred {
                if let Some(value) = completed.clauses.get(slot) {
                    if domains[*slot] == p(P::Type) {
                        let index = parameters.iter().position(|id| *id == all[*slot]).unwrap();
                        candidate
                            .constrain_type_argument(index, value, crate::ptc::Budget::default())
                            .map_err(|e| call_inference_error(span, e))?;
                    }
                    continue;
                }
                if self.checking_code_generator {
                    break;
                }
                let mut unknown = BTreeSet::new();
                let hints = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, id)| {
                        candidate
                            .type_argument(index)
                            .map(|value| {
                                value.unwrap_or_else(|| {
                                    unknown.insert(*id);
                                    Term::Rigid {
                                        declaration: *id,
                                        sort: self.input_binders(&[*id])[0].sort,
                                    }
                                })
                            })
                            .map_err(|e| call_inference_error(span, e))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let prefix = plan[..*slot]
                    .iter()
                    .map(|ty| {
                        ty.instantiate_parameters(&self.input_binders(parameters), &hints)
                            .map_err(|e| error(span, e.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ready = unknown.is_empty()
                    || self.with_generator_clause_default(
                        owner,
                        *slot,
                        &prefix,
                        true,
                        |this, expression, domain| {
                            if *domain != p(P::Type)
                                && !super::super::super::required::signature_inputs_ready_with_clauses(
                                    expression, 0, &|_| true, &|id| {
                                        !unknown.contains(&DeclarationId {
                                            package: semantic_package(&this.package_identities, id.pkg),
                                            local: id.index,
                                        })
                                    },
                                )? {
                                return Ok(false);
                            }
                            let mut checked = this.expr(expression, Some(domain))?;
                            let mut dependent = false;
                            checked.rewrite_types(&mut |ty| {
                                dependent |= unknown.iter().any(|id| ty.mentions_rigid(*id));
                                this.copy_alias_type(ty, span)
                            })?;
                            Ok(!dependent)
                        },
                    )?;
                if !ready {
                    break;
                }
                let value = self.generator_clause_default(owner, *slot, &prefix)?;
                if domains[*slot] == p(P::Type) {
                    let index = parameters.iter().position(|id| *id == all[*slot]).unwrap();
                    candidate
                        .constrain_type_argument(index, &value, crate::ptc::Budget::default())
                        .map_err(|e| call_inference_error(span, e))?;
                }
                plan[*slot] = self.copy_alias_type(&value, span)?;
                completed.clauses.insert(*slot, value);
            }
            if completed.types.len() + completed.clauses.len() == before {
                return Ok(candidate);
            }
        }
    }

    pub(in crate::program::cmtt) fn check_generator_clause_defaults(
        &mut self,
        owner: OwnerId,
        definition: &Generator,
    ) -> Result<(), Error> {
        self.check_generator_clause_value_defaults(owner, &definition.types)
    }

    pub(in crate::program::cmtt) fn check_generator_clause_value_defaults(
        &mut self,
        owner: OwnerId,
        types: &[(DeclarationId, Term)],
    ) -> Result<(), Error> {
        let values = types
            .iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        for (slot, clause) in self.type_clause_params(owner).iter().enumerate() {
            if matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)) {
                self.with_generator_clause_default(
                    owner,
                    slot,
                    &values[..slot],
                    true,
                    |this, expression, domain| {
                        let checked = this.expr(expression, Some(domain))?;
                        this.validate_generator_data(&checked, expression.span)
                    },
                )?;
            }
        }
        Ok(())
    }

    pub(in crate::program::cmtt) fn generator_clause_plan(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<GeneratorTypePlan, Error> {
        let domains = self.trait_domains(owner)?;
        if domains.iter().all(|ty| *ty == p(P::Type)) {
            let (parameters, values, pending) = self.function_type_plan(owner, &[], span)?;
            return Ok((parameters, values, pending, vec![]));
        }
        let clauses = self.type_clause_params(owner);
        let parameters = self.type_parameters[&owner].clone();
        let mut inferred = Vec::new();
        let mut values = Vec::new();
        let mut waiting = BTreeSet::new();
        let mut deferred = Vec::new();
        for (index, (clause, domain)) in clauses.iter().zip(&domains).enumerate() {
            if let hir::clause::ClauseParamKind::Optional(_, _, default) = clause.kind {
                let value_dependency = *domain != p(P::Type)
                    && !super::super::super::required::signature_inputs_ready_with_clauses(
                        default,
                        0,
                        &|_| true,
                        &|id| {
                            !waiting.contains(&DeclarationId {
                                package: semantic_package(&self.package_identities, id.pkg),
                                local: id.index,
                            })
                        },
                    )?;
                let must_wait = value_dependency
                    || !deferred.is_empty()
                    || (!waiting.is_empty()
                        && self.with_generator_clause_default(
                            owner,
                            index,
                            &values,
                            true,
                            |this, expression, domain| {
                                let mut checked = this.expr(expression, Some(domain))?;
                                let mut budget = comptime_runtime::Budget {
                                    steps: this.required_steps as usize,
                                    max_limbs: 4096,
                                };
                                let known = this.known_normal_input(&checked, 0, &mut budget);
                                this.required_steps = budget.steps as u64;
                                // Structural Type aliases preserve their relationship
                                // to earlier inputs without invoking a Type producer.
                                if known.is_some_and(|value| {
                                    *domain == p(P::Type)
                                        || !waiting.iter().any(|id| value.mentions_rigid(*id))
                                }) {
                                    return Ok(false);
                                }
                                let mut dependent = false;
                                checked.rewrite_types(&mut |ty| {
                                    dependent |= waiting.iter().any(|id| ty.mentions_rigid(*id));
                                    this.copy_alias_type(ty, expression.span)
                                })?;
                                let mut fuel = this.required_steps as usize;
                                let free = nir::free_bindings::collect(&checked, &mut fuel);
                                this.required_steps = fuel as u64;
                                let free = free.map_err(|reason| error(expression.span, reason))?;
                                dependent |=
                                    clauses[..index].iter().enumerate().any(|(slot, clause)| {
                                        waiting.contains(&parameters[slot])
                                            && free.contains(&local(clause.hir_id))
                                    });
                                Ok(dependent)
                            },
                        )?);
                if must_wait {
                    deferred.push(index);
                    waiting.insert(parameters[index]);
                    let sort = if *domain == p(P::Type) {
                        inferred.push(parameters[index]);
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    };
                    values.push(Term::Rigid {
                        declaration: parameters[index],
                        sort,
                    });
                } else {
                    values.push(self.generator_clause_default(owner, index, &values)?);
                }
            } else {
                inferred.push(parameters[index]);
                waiting.insert(parameters[index]);
                values.push(Term::Rigid {
                    declaration: parameters[index],
                    sort: if *domain == p(P::Type) {
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    },
                });
            }
        }
        Ok((inferred, values, vec![], deferred))
    }

    pub(in crate::program::cmtt) fn generator_clause_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        previous: &[Term],
    ) -> Result<Term, Error> {
        self.with_generator_clause_default(
            owner,
            slot,
            previous,
            false,
            |this, expression, domain| this.normal_value_argument(expression, domain),
        )
    }

    fn with_generator_clause_default<R>(
        &mut self,
        owner: OwnerId,
        slot: usize,
        previous: &[Term],
        probe: bool,
        check: impl FnOnce(&mut Self, &Expr<'_>, &Term) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let clauses = self.type_clause_params(owner);
        let hir::clause::ClauseParamKind::Optional(_, _, expression) = clauses[slot].kind else {
            return Err(error(clauses[slot].span, "generator clause has no default"));
        };
        let parameters = self.type_parameters[&owner].clone();
        let domains = self.trait_domains(owner)?;
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_aliases = self.alias_substitution.clone();
        let saved_forbidden = self.forbidden_default_inputs.clone();
        let saved_bindings = self.normal_default_bindings.clone();
        let saved_required = std::mem::replace(&mut self.in_required_comptime, true);
        let mut saved_inputs = Vec::new();
        self.context = self.definition_context(owner);
        if let Some(captures) = self.code_method_captures.get(&owner) {
            self.alias_substitution.extend(
                captures
                    .iter()
                    .map(|(id, value)| (*id, Arc::new(value.clone()))),
            );
        }
        for parameter in &parameters {
            self.normal_default_bindings.remove(parameter);
        }
        self.alias_substitution.extend(
            parameters
                .iter()
                .zip(previous)
                .map(|(id, value)| (*id, Arc::new(value.clone()))),
        );
        self.forbidden_default_inputs
            .extend(parameters[slot..].iter().copied());
        let result = (|| {
            if probe {
                for (index, clause) in clauses[..slot].iter().enumerate() {
                    if domains[index] == p(P::Type) {
                        continue;
                    }
                    let ty = self.substituted_trait_domain(&domains[index], expression.span)?;
                    let binding = local(clause.hir_id);
                    let old_local = self.locals.insert(binding, ty.clone());
                    let old_data = self.generator_data.insert(binding, None);
                    saved_inputs.push((binding, old_local, old_data));
                    self.normal_default_bindings
                        .insert(parameters[index], (binding, ty));
                }
            }
            let domain = self.substituted_trait_domain(&domains[slot], expression.span)?;
            check(self, expression, &domain)
        })();
        for (binding, local, data) in saved_inputs {
            if let Some(value) = local {
                self.locals.insert(binding, value);
            } else {
                self.locals.remove(&binding);
            }
            if let Some(value) = data {
                self.generator_data.insert(binding, value);
            } else {
                self.generator_data.remove(&binding);
            }
        }
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.alias_substitution = saved_aliases;
        self.forbidden_default_inputs = saved_forbidden;
        self.normal_default_bindings = saved_bindings;
        self.in_required_comptime = saved_required;
        result
    }
}
