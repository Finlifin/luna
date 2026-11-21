//! Shared clause packets, required input invocation, and staged comptime callables.
use super::*;

mod domain_checks;

/// A checked packet keeps executable producers separate from static identities.
/// A typed operand may establish a signature equality conditional on successful
/// admission. The pending producer still supplies the actual instance argument.
pub(super) struct CallableClausePacket {
    declaration: domain_checks::Declaration,
    inputs: n::ClauseOperands,
    bindings: declaration_inputs::CallableBindings,
    admitted: BTreeMap<DeclarationId, Term>,
    formation_deferred: bool,
}

impl std::fmt::Display for CallableClausePacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(callable-clause-packet {} {} {} (formation-deferred {}) (admitted-domains",
            self.inputs, self.bindings, self.declaration, self.formation_deferred
        )?;
        for (id, ty) in &self.admitted {
            write!(f, " ({id} {ty})")?;
        }
        f.write_str("))")
    }
}

/// Retained consumer of a clause preparation. Formation and final domain checks
/// must run in the restored caller scope before a callable packet can escape.
struct CallableCompletion {
    owner: OwnerId,
}

impl declaration_inputs::Completion for CallableCompletion {
    const CHECKING: declaration_inputs::InputChecking = declaration_inputs::InputChecking::Typed;

    type Output = CallableClausePacket;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        telescope: declaration_inputs::PreparedTelescope,
        inputs: declaration_inputs::PreparedDeclarationInputs,
        span: Span,
    ) -> Result<Self::Output, Error> {
        let (declaration, prefix) =
            domain_checks::Declaration::from_telescope(self.owner, telescope);
        let prepared = inputs.into_typed(&mut checker.required_steps, span)?;
        let (bindings, formation, prefix_operands) =
            declaration.bind_inputs(checker, prefix, prepared.observations, span)?;
        let mut inputs = prepared.operands;
        inputs.arguments.splice(..0, prefix_operands);
        let mut steps = checker.required_steps;
        let checked = declaration.check_domains(
            checker,
            &bindings,
            &formation,
            &inputs.arguments,
            &mut steps,
            span,
        );
        checker.required_steps = steps;
        let admitted = checked?;
        let formation_deferred = declaration.check_formation(checker, formation, span)?;
        Ok(CallableClausePacket {
            declaration,
            inputs,
            bindings,
            admitted,
            formation_deferred,
        })
    }
}

impl domain_checks::Declaration {
    fn bind_inputs(
        &self,
        checker: &mut Checker<'_, '_>,
        prefix: Vec<Term>,
        observations: declaration_inputs::Observations,
        span: Span,
    ) -> Result<
        (
            declaration_inputs::CallableBindings,
            FormationInputs,
            Vec<n::Expr>,
        ),
        Error,
    > {
        let prefix_count = prefix.len();
        let parent_inputs = self
            .parameters
            .iter()
            .zip(&self.domains)
            .zip(prefix)
            .map(|((&id, domain), value)| (nir::DeclarationParameter::comptime(id, domain), value));
        let observations = observations.with_prefix(parent_inputs);
        let frames = checker.call_input_frames();
        let (bindings, formation) =
            observations.bind(&self.domains, frames, &mut checker.required_steps, span)?;
        let mut prefix_operands = Vec::with_capacity(prefix_count);
        for (domain, value) in self.domains[..prefix_count].iter().zip(&formation.terms) {
            let ty = bindings.instantiate(domain, &mut checker.required_steps, span)?;
            let value = checker.copy_alias_type(value, span)?;
            prefix_operands.push(n::Expr {
                source: source(span),
                kind: if ty == p(P::Type) {
                    K::TypeValue(value)
                } else {
                    K::ComptimeValue(value)
                },
                ty,
            });
        }
        Ok((bindings, formation, prefix_operands))
    }

    fn check_formation(
        &self,
        checker: &mut Checker<'_, '_>,
        mut formation: FormationInputs,
        span: Span,
    ) -> Result<bool, Error> {
        formation.deferred.extend(
            self.parameters
                .iter()
                .zip(&self.domains)
                .filter_map(|(id, domain)| (*domain != p(P::Type)).then_some(*id)),
        );
        let formation_deferred = !formation.deferred.is_empty();
        checker.check_formation_application(self.owner, formation, span)?;
        Ok(formation_deferred)
    }
}

impl std::fmt::Display for CallableCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(callable-input-completion (owner {}))", self.owner)
    }
}

/// The final consumer survives every clause-input child check. No signature is
/// published before both the packet and its declaration-side checks complete.
pub(super) struct InstanceCompletion<'h> {
    packet: CallableCompletion,
    expr: Expr<'h>,
    signature: Term,
    expected: Option<Term>,
    allow_optional: bool,
    delivery: Option<bool>,
}

impl declaration_inputs::Completion for InstanceCompletion<'_> {
    const CHECKING: declaration_inputs::InputChecking =
        declaration_inputs::InputChecking::Contextual;

    type Output = n::Expr;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        telescope: declaration_inputs::PreparedTelescope,
        inputs: declaration_inputs::PreparedDeclarationInputs,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let owner = self.packet.owner;
        let value = if inputs
            .arguments
            .iter()
            .all(|arg| matches!(arg, n::NormalFormArgument::Value(_)))
        {
            let packet = self.packet.finish(checker, telescope, inputs, span)?;
            checker.finish_comptime_function_instance(
                &self.expr,
                owner,
                self.signature,
                packet,
                self.expected.as_ref(),
                self.allow_optional,
            )?
        } else {
            let (contract, prefix) = domain_checks::Declaration::from_telescope(owner, telescope);
            let declaration_inputs::PreparedDeclarationInputs {
                mut plan,
                mut arguments,
                observations,
            } = inputs;
            let (bindings, formation, prefix) =
                contract.bind_inputs(checker, prefix, observations, span)?;
            let count = prefix.len();
            for index in &mut plan.order {
                *index += count;
            }
            for preparation in &mut plan.preparations {
                preparation.before += count;
            }
            plan.order.splice(..0, 0..count);
            let captured = (0..count)
                .map(|_| checker.spread_binding(span))
                .collect::<Result<Vec<_>, _>>()?;
            plan.bindings.splice(..0, captured);
            arguments.splice(..0, prefix.into_iter().map(n::NormalFormArgument::Value));
            let mut steps = checker.required_steps;
            let checked = contract.check_contextual_domains(
                checker, &bindings, &formation, &arguments, &mut steps, span,
            );
            checker.required_steps = steps;
            let admitted = checked?;
            contract.check_formation(checker, formation, span)?;
            let signature = bindings.admitted_signature(
                checker,
                &self.signature,
                &admitted,
                contract.parameters.iter().copied().zip(&contract.domains),
                span,
            )?;
            let ty = checker.callable_type(owner, signature);
            if let Some(expected) = &self.expected {
                checker.require(&ty, expected, span)?;
            }
            n::Expr {
                source: source(span),
                ty,
                kind: K::PlannedFunctionInstance {
                    declaration: declaration(checker.package, &checker.package_identities, owner),
                    arguments,
                    plan: Some(plan),
                },
            }
        };
        let Some(preserve_effect) = self.delivery else {
            return Ok(value);
        };
        let value = checker.callable_value_view(value, self.expected.as_ref(), span)?;
        if preserve_effect {
            Ok(value)
        } else {
            checker.run_computation(value, self.expected.as_ref(), span)
        }
    }
}

impl std::fmt::Display for InstanceCompletion<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(callable-instance-completion (source {} {}) {} (signature {}) (expected",
            self.expr.hir_id,
            source(self.expr.span),
            self.packet,
            self.signature
        )?;
        if let Some(expected) = &self.expected {
            write!(f, " {expected}")?;
        }
        write!(
            f,
            ") (allow-optional {}) (delivery {}))",
            self.allow_optional,
            match self.delivery {
                None => "caller",
                Some(true) => "preserve",
                Some(false) => "consume",
            }
        )
    }
}

impl<'a, 'arena> Checker<'a, 'arena> {
    pub(super) fn prepared_callable_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: &[Term],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<Option<n::Expr>, Error> {
        if let Some(value) =
            self.comptime_function_instance(expr, owner, args, prefix, expected, allow_optional)?
        {
            return Ok(Some(value));
        }
        self.required_collected_callable_instance(
            expr,
            owner,
            args,
            prefix,
            expected,
            allow_optional,
        )
    }

    pub(super) fn required_collected_callable_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: &[Term],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<Option<n::Expr>, Error> {
        if self.clause_collector(owner).is_none() || self.in_code_body {
            return Ok(None);
        }
        let inputs = self.required_callable_clause_arguments(expr, owner, args, prefix)?;
        self.function_instance(expr, owner, inputs, expected, allow_optional)
            .map(Some)
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
        let Some(preparation) = self.begin_comptime_function_instance(
            expr,
            owner,
            args,
            prefix,
            expected,
            allow_optional,
            None,
        )?
        else {
            return Ok(None);
        };
        self.check_declaration_inputs_region(preparation).map(Some)
    }

    pub(super) fn begin_comptime_function_instance<'h>(
        &mut self,
        expr: &Expr<'h>,
        owner: OwnerId,
        args: &[Arg<'h>],
        prefix: &[Term],
        expected: Option<&Term>,
        allow_optional: bool,
        delivery: Option<bool>,
    ) -> Result<Option<declaration_inputs::preparation::Frame<'h, InstanceCompletion<'h>>>, Error>
    where
        'a: 'h,
        'arena: 'h,
    {
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
        // Only the prepared inputs can say whether the result interface is
        // known. A value binder in its declaration may already have an identity
        // in this call, even when its producer has effects before returning.
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
        let domains = self.trait_domains(owner)?;
        let Some(signature) = self.declared_signature(owner).map(|sig| sig.into_owned()) else {
            return Ok(None);
        };
        let (captured, local_domains) = declaration_inputs::KnownInputs::partition(
            &parameters,
            domains,
            prefix,
            &mut self.required_steps,
            expr.span,
        )?;
        if !allow_optional {
            arguments::require_function_value_parameters(sig.params, expr.span)?;
        }
        let packet = CallableCompletion { owner };
        let expected = expected
            .map(|ty| self.copy_alias_type(ty, expr.span))
            .transpose()?;
        let completion = InstanceCompletion {
            packet,
            expr: expr.clone(),
            signature,
            expected,
            allow_optional,
            delivery,
        };
        self.begin_callable_clause_packet(
            expr,
            owner,
            args,
            captured,
            &parameters,
            local_domains,
            completion,
        )
        .map(Some)
    }

    fn finish_comptime_function_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        signature: Term,
        packet: CallableClausePacket,
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<n::Expr, Error> {
        if !packet.formation_deferred && packet.inputs.statements.is_empty() {
            // Only an already checked static application can use the direct
            // instance and declaration-side default substitution. An observed
            // identity cannot erase producers or complete deferred contracts.
            return self.function_instance(
                expr,
                owner,
                packet
                    .bindings
                    .into_identities()
                    .expect("formation has no deferred inputs"),
                expected,
                allow_optional,
            );
        }
        let signature = packet.bindings.admitted_signature(
            self,
            &signature,
            &packet.admitted,
            packet
                .declaration
                .parameters
                .iter()
                .copied()
                .zip(&packet.declaration.domains),
            expr.span,
        )?;
        let ty = self.callable_type(owner, signature);
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        let mut inputs = packet.inputs;
        packet.bindings.move_observations(
            &packet.declaration.parameters,
            &mut inputs,
            &mut self.required_steps,
            expr.span,
        )?;
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ComptimeFunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                inputs,
            },
        })
    }

    /// Prepare ordinary clause values without choosing their execution time or
    /// the phase of the eventual callable. The packet is an internal carrier;
    /// every operand retains its declaration domain and source-order producer.
    fn prepare_callable_clause_packet(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: &[Term],
        parameters: &[DeclarationId],
        domains: Vec<Term>,
    ) -> Result<CallableClausePacket, Error> {
        let completion = CallableCompletion { owner };
        let (captured, local_domains) = declaration_inputs::KnownInputs::partition(
            parameters,
            domains,
            prefix,
            &mut self.required_steps,
            expr.span,
        )?;
        let preparation = self.begin_callable_clause_packet(
            expr,
            owner,
            args,
            captured,
            parameters,
            local_domains,
            completion,
        )?;
        self.check_declaration_inputs_region(preparation)
    }

    fn begin_callable_clause_packet<'h, C: declaration_inputs::Completion>(
        &mut self,
        expr: &Expr<'h>,
        owner: OwnerId,
        args: &[Arg<'h>],
        captured: declaration_inputs::KnownInputs,
        parameters: &[DeclarationId],
        local_domains: Vec<Term>,
        completion: C,
    ) -> Result<declaration_inputs::preparation::Frame<'h, C>, Error>
    where
        'a: 'h,
        'arena: 'h,
    {
        let prefix = captured.values();
        // Prefix identities specialize declaration defaults, never explicit
        // caller sources. Keep one budgeted owned environment, not a temporary
        // unbounded clone followed by another retention inside preparation.
        let defaults = declaration_inputs::Environment::retain_prefix(
            &nominal::InputEnvironment {
                substitution: &self.alias_substitution,
                bindings: &self.alias_value_bindings,
                normal_bindings: &self.normal_default_bindings,
                context: &self.context,
            },
            parameters,
            prefix,
            &mut self.required_steps,
            expr.span,
        )?;
        let telescope = parameters[prefix.len()..]
            .iter()
            .zip(local_domains)
            .map(|(&id, domain)| (id, Sort::of_comptime_domain(&domain), domain))
            .collect();
        declaration_inputs::preparation::Frame::begin(
            self,
            owner,
            args,
            expr.span,
            telescope,
            captured,
            Some(defaults),
            completion,
        )
    }

    pub(super) fn function_clause_arguments(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
    ) -> Result<Vec<Term>, Error> {
        let prefix = self.lexical_function_arguments(owner, expr.span)?;
        if self.clause_collector(owner).is_some() {
            self.required_callable_clause_arguments(expr, owner, args, &prefix)
        } else {
            self.type_clause_arguments_with_prefix(owner, args, None, expr.span, prefix)
        }
    }

    /// Execute only the prepared input packet, then discharge its domain checks
    /// with the completed arguments. The eventual runtime callable still needs
    /// ordinary formation checks; this operation never invokes its body.
    pub(super) fn required_callable_clause_arguments(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: &[Term],
    ) -> Result<Vec<Term>, Error> {
        let parameters = self.type_parameters[&owner].clone();
        let domains = self.trait_domains(owner)?;
        let (prepared, declaration) = self.prepare_required_root_with_live_inputs(
            expr,
            required::RootKind::CallableInputs,
            Vec::new(),
            false,
            &[],
            move |checker| {
                let packet = checker.prepare_callable_clause_packet(
                    expr,
                    owner,
                    args,
                    prefix,
                    &parameters,
                    domains,
                )?;
                // A required invocation returns one value to the host.
                let n::ClauseOperands {
                    statements,
                    arguments,
                    ..
                } = packet.inputs;
                let ty = Term::Tuple(
                    arguments
                        .iter()
                        .map(|arg| checker.copy_alias_type(&arg.ty, expr.span))
                        .collect::<Result<_, _>>()?,
                );
                Ok((
                    n::Expr {
                        source: source(expr.span),
                        ty: checker.copy_alias_type(&ty, expr.span)?,
                        kind: K::Block {
                            statements,
                            result: Some(Box::new(n::Expr {
                                source: source(expr.span),
                                ty,
                                kind: K::Tuple(arguments),
                            })),
                        },
                    },
                    packet.declaration,
                ))
            },
        )?;
        let (value, packet_type) =
            self.invoke_required_publishing(prepared, Vec::new(), expr.hir_id.owner, expr.span)?;
        let comptime_interpretor::Value::Tuple(values) = value else {
            return Err(error(
                expr.span,
                "clause input packet did not produce a tuple",
            ));
        };
        if values.len() != declaration.parameters.len() {
            return Err(error(expr.span, "clause input packet arity mismatch"));
        }
        let arguments = values
            .into_iter()
            .map(|value| match value {
                comptime_interpretor::Value::TypeParameter(declaration) => Ok(Term::Rigid {
                    declaration,
                    sort: Sort::Type,
                }),
                value => self.required_normal_term(value, expr.span),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let Term::Tuple(actual_domains) = packet_type else {
            return Err(error(
                expr.span,
                "clause input packet has no checked domains",
            ));
        };
        declaration.check_completed(self, &actual_domains, &arguments, expr.span)?;
        Ok(arguments)
    }
}

#[cfg(test)]
mod packet_tests {
    use super::*;

    #[test]
    fn callable_completion_dump_identifies_the_consumer_and_signature() {
        let completion = CallableCompletion {
            owner: hir::OwnerId::new(hir::LocalDefId::new(3)),
        };
        let instance = InstanceCompletion {
            expr: Expr {
                hir_id: hir::HirId::new(completion.owner, hir::ItemLocalId::new(0)),
                span: Span::default(),
                kind: H::Unit,
            },
            packet: completion,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(p(P::Bool)),
            },
            expected: None,
            allow_optional: false,
            delivery: Some(true),
        };
        let dump = instance.to_string();
        assert!(dump.starts_with("(callable-instance-completion"), "{dump}");
        assert!(dump.contains("(signature (function"), "{dump}");
        assert!(
            dump.contains("(allow-optional false) (delivery preserve)"),
            "{dump}"
        );
        assert!(
            dump.contains("(callable-input-completion (owner "),
            "{dump}"
        );
        assert!(!dump.contains('\n'), "{dump}");
    }

    #[test]
    fn callable_packet_dump_retains_producer_and_orders_pending_identities() {
        let first = DeclarationId {
            package: 1,
            local: 2,
        };
        let last = DeclarationId {
            package: 1,
            local: 9,
        };
        let expression = n::Expr {
            source: n::SourceRange { start: 3, end: 5 },
            ty: p(P::Bool),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let producer = expression.to_string();
        let packet = CallableClausePacket {
            declaration: domain_checks::Declaration {
                owner: hir::OwnerId::new(hir::LocalDefId::new(3)),
                parameters: vec![last, first],
                domains: vec![p(P::Type), p(P::Type)],
            },
            inputs: n::ClauseOperands {
                observations: Default::default(),
                statements: vec![],
                arguments: vec![expression],
            },
            bindings: declaration_inputs::Observations::new(
                vec![last, first]
                    .into_iter()
                    .map(|declaration| {
                        (
                            nir::DeclarationParameter {
                                declaration,
                                sort: Sort::Type,
                            },
                            None,
                        )
                    })
                    .collect(),
            )
            .bind(
                &[p(P::Type), p(P::Type)],
                vec![],
                &mut 1000,
                Span::default(),
            )
            .unwrap()
            .0,
            admitted: BTreeMap::from([(last, p(P::Bool))]),
            formation_deferred: true,
        };
        let dump = packet.to_string();
        assert!(dump.contains(&producer));
        assert!(dump.contains(&format!("(pending {first} {last})")));
        assert!(dump.contains(&format!("(admitted-domains ({last} {}))", p(P::Bool))));
        assert!(dump.find(&first.to_string()) < dump.find(&last.to_string()));
    }
}
