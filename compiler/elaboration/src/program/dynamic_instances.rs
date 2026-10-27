//! Invocation-time clause inputs for already-known comptime callable signatures.
use super::*;

impl Checker<'_, '_> {
    /// Definition-side evidence only: pending values retain declaration rigids.
    /// The invocation packet remains the authority for their actual snapshots.
    pub(super) fn comptime_instance_clause_evidence(
        &self,
        owner: OwnerId,
        subject: &n::Expr,
    ) -> Option<Vec<Term>> {
        let packet = match &subject.kind {
            K::Block {
                result: Some(result),
                ..
            } => result.as_ref(),
            _ => subject,
        };
        let K::Tuple(inputs) = &packet.kind else {
            return None;
        };
        let parameters = self.type_parameters.get(&owner)?;
        if inputs.len() != parameters.len() {
            return None;
        }
        Some(
            parameters
                .iter()
                .zip(inputs)
                .map(|(id, input)| match &input.kind {
                    K::TypeValue(ty) | K::ComptimeValue(ty) => ty.clone(),
                    _ => Term::Rigid {
                        declaration: *id,
                        sort: Sort::ComptimeValue,
                    },
                })
                .collect(),
        )
    }

    pub(super) fn comptime_function_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: &[Term],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<Option<n::Expr>, Error> {
        if self.runtime_value_context() || self.in_code_body {
            return Ok(None);
        }
        let Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(None);
        };
        if !sig.modifiers.is_comptime
            || sig.modifiers.is_normal_form_head
            || self.open_functions.contains_key(&owner)
        {
            return Ok(None);
        }
        // Value-dependent callable signatures still need a separate invocation
        // plan before their callable type can be formed.
        let parameters = self
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if parameters.is_empty()
            || self.type_clause_params(owner).len() + prefix.len() != parameters.len()
            || !args
                .iter()
                .all(|arg| matches!(arg, Arg::Positional(_) | Arg::Named(..) | Arg::Expand(_)))
        {
            return Ok(None);
        }
        let declared_domains = self.trait_domains(owner)?;
        let prefix_binders = self.input_binders(&parameters[..prefix.len()]);
        let domains = declared_domains
            .iter()
            .map(|domain| {
                domain
                    .instantiate_parameters_in_scope(
                        &prefix_binders,
                        prefix,
                        &self.call_input_frames(),
                    )
                    .map_err(|failure| error(expr.span, failure.to_string()))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        if domains.iter().all(|domain| *domain == p(P::Type)) {
            return Ok(None);
        }
        let Some(signature) = self.known_callable_signature(owner) else {
            return Ok(None);
        };
        let signature = signature
            .instantiate_parameters_in_scope(&prefix_binders, prefix, &self.call_input_frames())
            .map_err(|failure| error(expr.span, failure.to_string()))?;
        let value_parameters: Vec<_> = parameters
            .iter()
            .zip(&domains)
            .filter_map(|(id, domain)| (*domain != p(P::Type)).then_some(*id))
            .collect();
        if value_parameters.iter().any(|id| {
            signature.mentions_rigid(*id) || domains.iter().any(|domain| domain.mentions_rigid(*id))
        }) {
            return Ok(None);
        }
        if !allow_optional {
            arguments::require_function_value_parameters(sig.params, expr.span)?;
        }
        let saved_locals = self.locals.clone();
        let saved_substitution = self.alias_substitution.clone();
        let saved_bindings = self.alias_value_bindings.clone();
        let saved_normal_bindings = self.normal_default_bindings.clone();
        let saved_owner = self.signature_owner;
        let saved_context = self.context.clone();
        let saved_mode = self.in_required_comptime;
        let prepared = (|| {
            let clauses = self.type_clause_params(owner);
            let count = clauses
                .iter()
                .filter(|clause| !matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
                .count();
            let mut statements = Vec::new();
            let inputs = self.prepared_clause_inputs_in_domains(
                owner,
                args,
                count,
                &mut statements,
                expr.span,
                &domains[prefix.len()..],
                &parameters[prefix.len()..],
            )?;
            let mut supplied = vec![None; clauses.len()];
            let mut positional = 0;
            for (name, value) in inputs {
                let slot = Self::type_input_slot(&clauses, name, &mut positional, expr.span)?;
                let cell = supplied
                    .get_mut(slot)
                    .ok_or_else(|| error(expr.span, "type argument count mismatch"))?;
                if cell.replace(value).is_some() {
                    return Err(error(expr.span, "duplicate type argument"));
                }
            }
            self.signature_owner = Some(owner);
            self.context = self.binder_context(owner);
            self.in_required_comptime = true;
            let mut operands = Vec::new();
            let mut symbols = prefix.to_vec();
            for (slot, term) in prefix.iter().enumerate() {
                self.alias_substitution
                    .insert(parameters[slot], std::sync::Arc::new(term.clone()));
                self.alias_value_bindings.remove(&parameters[slot]);
                self.normal_default_bindings.remove(&parameters[slot]);
                operands.push(n::Expr {
                    source: source(expr.span),
                    ty: domains[slot].clone(),
                    kind: if domains[slot] == p(P::Type) {
                        K::TypeValue(term.clone())
                    } else {
                        K::ComptimeValue(term.clone())
                    },
                });
            }
            for (own_slot, supplied) in supplied.into_iter().enumerate() {
                let slot = prefix.len() + own_slot;
                let value = if let Some(value) = supplied {
                    value
                } else {
                    let hir::clause::ClauseParamKind::Optional(_, _, default) =
                        clauses[own_slot].kind
                    else {
                        return Err(error(expr.span, "type argument count mismatch"));
                    };
                    if self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, own_slot))
                    {
                        return Err(error(expr.span, "cyclic or excessively deep type default"));
                    }
                    let value = (|| {
                        let domain = self.substituted_trait_domain(&domains[slot], expr.span)?;
                        self.expr(default, Some(&domain))
                    })();
                    self.active_type_defaults.remove(&(owner, own_slot));
                    self.bind_type_input(value?, &mut statements, expr.span)?
                };
                match &value.kind {
                    K::TypeValue(ty) => {
                        symbols.push(ty.clone());
                        self.alias_substitution
                            .insert(parameters[slot], std::sync::Arc::new(ty.clone()));
                        self.alias_value_bindings.remove(&parameters[slot]);
                    }
                    K::Local(binding) if domains[slot] != p(P::Type) => {
                        symbols.push(Term::Rigid {
                            declaration: parameters[slot],
                            sort: Sort::ComptimeValue,
                        });
                        self.normal_default_bindings
                            .insert(parameters[slot], (*binding, value.ty.clone()));
                        self.alias_value_bindings.remove(&parameters[slot]);
                        self.alias_substitution.remove(&parameters[slot]);
                    }
                    _ => {
                        return Err(error(
                            expr.span,
                            "comptime callable signature requires a known Type clause input",
                        ));
                    }
                }
                operands.push(value);
            }
            Ok((operands, symbols, statements))
        })();
        self.locals = saved_locals;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_bindings;
        self.normal_default_bindings = saved_normal_bindings;
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.in_required_comptime = saved_mode;
        let (operands, symbols, statements) = prepared?;
        self.check_formation_application(
            owner,
            FormationInputs {
                terms: symbols.clone(),
                deferred: value_parameters.into_iter().collect(),
                environment: BTreeMap::new(),
            },
            expr.span,
        )?;
        // Named Type inputs may occur after their value inputs. Once routing
        // completes, validate every retained value against the completed domain
        // even if this function is never invoked.
        for (operand, domain) in operands.iter().zip(&domains) {
            let domain = domain
                .instantiate_parameters_in_scope(
                    &self.input_binders(&parameters),
                    &symbols,
                    &self.call_input_frames(),
                )
                .map_err(|failure| error(expr.span, failure.to_string()))?;
            self.require(&operand.ty, &domain, expr.span)?;
        }
        let signature = signature
            .instantiate_parameters_in_scope(
                &self.input_binders(&parameters),
                &symbols,
                &self.call_input_frames(),
            )
            .map_err(|failure| error(expr.span, failure.to_string()))?;
        let ty = self.callable_type(owner, signature);
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(Some(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ComptimeFunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                subject: Box::new(n::Expr {
                    source: source(expr.span),
                    ty: Term::Tuple(operands.iter().map(|value| value.ty.clone()).collect()),
                    kind: K::Block {
                        statements,
                        result: Some(Box::new(n::Expr {
                            source: source(expr.span),
                            ty: Term::Tuple(
                                operands.iter().map(|value| value.ty.clone()).collect(),
                            ),
                            kind: K::Tuple(operands),
                        })),
                    },
                }),
            },
        }))
    }
}
