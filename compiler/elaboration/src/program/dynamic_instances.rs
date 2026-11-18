//! Shared clause packets, required input invocation, and staged comptime callables.
use super::*;

mod domain_checks;

/// A checked packet keeps executable producers separate from static identities.
/// Pending inputs can be supplied during invocation only when the consumer does
/// not publish a callable signature that still depends on those identities.
pub(super) struct CallableClausePacket {
    inputs: n::ClauseOperands,
    bindings: declaration_inputs::CallableBindings,
    formation_deferred: bool,
}

impl std::fmt::Display for CallableClausePacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(callable-clause-packet {} {} (formation-deferred {}))",
            self.inputs, self.bindings, self.formation_deferred
        )
    }
}

/// Retained consumer of a clause preparation. Formation and final domain checks
/// must run in the restored caller scope before a callable packet can escape.
struct CallableCompletion {
    owner: OwnerId,
    parameters: Vec<DeclarationId>,
    domains: Vec<Term>,
    prefix: Vec<Term>,
}

impl CallableCompletion {
    fn retain(
        checker: &mut Checker<'_, '_>,
        owner: OwnerId,
        parameters: &[DeclarationId],
        domains: &[Term],
        prefix: &[Term],
        span: Span,
    ) -> Result<Self, Error> {
        if parameters.len() != domains.len() || prefix.len() > parameters.len() {
            return Err(error(span, "clause input prefix arity mismatch"));
        }
        checker.required_steps = checker
            .required_steps
            .checked_sub(parameters.len() as u64)
            .ok_or_else(|| error(span, "callable input retention budget exhausted"))?;
        let mut retained_domains = Vec::new();
        for domain in domains {
            retained_domains.push(checker.copy_alias_type(domain, span)?);
        }
        let mut retained_prefix = Vec::new();
        for value in prefix {
            retained_prefix.push(checker.copy_alias_type(value, span)?);
        }
        Ok(Self {
            owner,
            parameters: parameters.to_vec(),
            domains: retained_domains,
            prefix: retained_prefix,
        })
    }
}

impl declaration_inputs::Completion for CallableCompletion {
    const CHECKING: declaration_inputs::InputChecking = declaration_inputs::InputChecking::Typed;
    type Output = CallableClausePacket;

    fn finish(
        mut self,
        checker: &mut Checker<'_, '_>,
        _: &[(DeclarationId, Sort, Term)],
        inputs: declaration_inputs::PreparedDeclarationInputs,
        span: Span,
    ) -> Result<Self::Output, Error> {
        let prefix = std::mem::take(&mut self.prefix);
        let parameters = &self.parameters;
        let domains = &self.domains;
        let prepared = inputs.into_typed(&mut checker.required_steps, span)?;
        let prefix_count = prefix.len();
        let prefix_binders = checker.input_binders(&parameters[..prefix_count]);
        let observations = prepared
            .observations
            .with_prefix(prefix_binders.into_iter().zip(prefix));
        let frames = checker.call_input_frames();
        let (bindings, mut formation) =
            observations.bind(domains, frames, &mut checker.required_steps, span)?;
        let mut prefix_operands = Vec::with_capacity(prefix_count);
        for (domain, value) in domains[..prefix_count].iter().zip(&formation.terms) {
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
        let mut inputs = prepared.operands;
        inputs.arguments.splice(..0, prefix_operands);
        let mut steps = checker.required_steps;
        let checked = self.check_domains(
            checker,
            &bindings,
            &formation,
            &inputs.arguments,
            &mut steps,
            span,
        );
        checker.required_steps = steps;
        checked?;
        formation.deferred.extend(
            parameters
                .iter()
                .zip(domains)
                .filter_map(|(id, domain)| (*domain != p(P::Type)).then_some(*id)),
        );
        let formation_deferred = !formation.deferred.is_empty();
        checker.check_formation_application(self.owner, formation, span)?;
        Ok(CallableClausePacket {
            inputs,
            bindings,
            formation_deferred,
        })
    }
}

impl std::fmt::Display for CallableCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(callable-input-completion (owner {}) (parameters",
            self.owner
        )?;
        for (parameter, domain) in self.parameters.iter().zip(&self.domains) {
            write!(f, " (input {parameter} {domain})")?;
        }
        f.write_str(") (prefix")?;
        for value in &self.prefix {
            write!(f, " {value}")?;
        }
        f.write_str("))")
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
    const CHECKING: declaration_inputs::InputChecking = declaration_inputs::InputChecking::Typed;
    type Output = n::Expr;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        telescope: &[(DeclarationId, Sort, Term)],
        inputs: declaration_inputs::PreparedDeclarationInputs,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let owner = self.packet.owner;
        let packet = self.packet.finish(checker, telescope, inputs, span)?;
        let value = checker.finish_comptime_function_instance(
            &self.expr,
            owner,
            self.signature,
            packet,
            self.expected.as_ref(),
            self.allow_optional,
        )?;
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
    /// Definition-side evidence only: pending values retain declaration rigids.
    /// The invocation packet remains the authority for their actual snapshots.
    pub(super) fn comptime_instance_clause_evidence(
        &self,
        owner: OwnerId,
        inputs: &n::ClauseOperands,
    ) -> Option<Vec<Term>> {
        let inputs = &inputs.arguments;
        let parameters = self.type_parameters.get(&owner)?;
        if inputs.len() != parameters.len() {
            return None;
        }
        Some(
            self.input_binders(parameters)
                .into_iter()
                .zip(inputs)
                .map(|(parameter, input)| match &input.kind {
                    K::TypeValue(ty) | K::ComptimeValue(ty) => ty.clone(),
                    _ => Term::Rigid {
                        declaration: parameter.declaration,
                        sort: parameter.sort,
                    },
                })
                .collect(),
        )
    }

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
        if !self.clause_has_collector(owner) || self.in_code_body {
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
        // Dependencies inside the clause telescope are checked while preparing
        // its packet. Only a value-dependent callable signature itself requires
        // a separate invocation plan before the callable type can be formed.
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
        let Some(signature) = self.known_callable_signature(owner) else {
            return Ok(None);
        };
        let declared_inputs = self.term_declaration_inputs(&signature, expr.span)?;
        let captured = declaration_inputs::KnownInputs::prefix(
            &parameters,
            &domains,
            prefix,
            &mut self.required_steps,
            expr.span,
        )?;
        for (id, domain) in parameters.iter().zip(&domains).skip(prefix.len()) {
            if declared_inputs.contains(id)
                && *domain != p(P::Type)
                && (prefix.is_empty()
                    || captured.instantiate(self, domain, expr.span)? != p(P::Type))
            {
                return Ok(None);
            }
        }
        if !allow_optional {
            arguments::require_function_value_parameters(sig.params, expr.span)?;
        }
        let packet =
            CallableCompletion::retain(self, owner, &parameters, &domains, prefix, expr.span)?;
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
            &domains,
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
        if packet
            .bindings
            .awaits_input(&signature, &mut self.required_steps, expr.span)?
        {
            return Err(error(
                expr.span,
                "comptime callable signature still depends on an invocation-time clause input",
            ));
        }
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
        let signature =
            packet
                .bindings
                .instantiate(&signature, &mut self.required_steps, expr.span)?;
        let ty = self.callable_type(owner, signature);
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ComptimeFunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                inputs: packet.inputs,
            },
        })
    }

    /// Prepare ordinary clause values without choosing their execution time or
    /// the phase of the eventual callable. The packet is an internal carrier;
    /// every operand retains its declaration domain and source-order producer.
    pub(super) fn prepare_callable_clause_packet(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        prefix: &[Term],
        parameters: &[DeclarationId],
        domains: &[Term],
    ) -> Result<CallableClausePacket, Error> {
        let completion =
            CallableCompletion::retain(self, owner, parameters, domains, prefix, expr.span)?;
        let captured = declaration_inputs::KnownInputs::prefix(
            parameters,
            domains,
            prefix,
            &mut self.required_steps,
            expr.span,
        )?;
        let preparation = self.begin_callable_clause_packet(
            expr, owner, args, captured, parameters, domains, completion,
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
        domains: &[Term],
        completion: C,
    ) -> Result<declaration_inputs::preparation::Frame<'h, C>, Error>
    where
        'a: 'h,
        'arena: 'h,
    {
        let prefix = captured.values();
        if parameters.len() != domains.len()
            || prefix.len() + self.type_clause_params(owner).len() != parameters.len()
        {
            return Err(error(expr.span, "clause input prefix arity mismatch"));
        }
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
        let mut telescope = Vec::new();
        for (binder, domain) in self
            .input_binders(&parameters[prefix.len()..])
            .into_iter()
            .zip(&domains[prefix.len()..])
        {
            telescope.push((
                binder.declaration,
                binder.sort,
                self.copy_alias_type(domain, expr.span)?,
            ));
        }
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
        if self.clause_has_collector(owner) {
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
        let prepared = self.prepare_required_root_with_live_inputs(
            expr,
            required::RootKind::CallableInputs,
            Vec::new(),
            false,
            &[],
            |checker| {
                let packet = checker.prepare_callable_clause_packet(
                    expr,
                    owner,
                    args,
                    prefix,
                    &parameters,
                    &domains,
                )?;
                // A required invocation returns one value to the host.
                let n::ClauseOperands {
                    statements,
                    arguments,
                } = packet.inputs;
                let ty = Term::Tuple(
                    arguments
                        .iter()
                        .map(|arg| checker.copy_alias_type(&arg.ty, expr.span))
                        .collect::<Result<_, _>>()?,
                );
                Ok(n::Expr {
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
                })
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
        if values.len() != parameters.len() {
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
        if actual_domains.len() != domains.len() {
            return Err(error(
                expr.span,
                "clause input packet domain arity mismatch",
            ));
        }
        // Required input execution returns the original checked packet's types.
        // Its completed values now substitute every declaration input at once;
        // no producer is inspected again or rerun to recover this evidence.
        for (actual, domain) in actual_domains.iter().zip(&domains) {
            let expected =
                self.instantiate_input_types(domain, &parameters, &arguments, expr.span)?;
            self.require(actual, &expected, expr.span)?;
        }
        Ok(arguments)
    }
}

#[cfg(test)]
mod packet_tests {
    use super::*;

    #[test]
    fn callable_completion_dump_keeps_parent_prefix_and_full_domains() {
        let parameter = DeclarationId {
            package: 1,
            local: 2,
        };
        let completion = CallableCompletion {
            owner: hir::OwnerId::new(hir::LocalDefId::new(3)),
            parameters: vec![parameter],
            domains: vec![p(P::Str)],
            prefix: vec![Term::String("parent\"\n".into())],
        };
        let dump = completion.to_string();
        assert!(
            dump.starts_with("(callable-input-completion (owner "),
            "{dump}"
        );
        assert!(
            dump.contains(&format!("(input {parameter} {})", p(P::Str))),
            "{dump}"
        );
        assert!(dump.contains("parent\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
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
        assert!(dump.contains("parent\\\"\\n"), "{dump}");
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
            inputs: n::ClauseOperands {
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
            formation_deferred: true,
        };
        let dump = packet.to_string();
        assert!(dump.contains(&producer));
        assert!(dump.contains(&format!("(pending {first} {last})")));
        assert!(dump.find(&first.to_string()) < dump.find(&last.to_string()));
    }
}
