//! Shared clause packets, required input invocation, and staged comptime callables.
use super::*;

/// A checked packet keeps executable producers separate from static identities.
/// Pending inputs can be supplied during invocation only when the consumer does
/// not publish a callable signature that still depends on those identities.
pub(super) struct CallableClausePacket {
    expression: n::Expr,
    identities: Vec<Term>,
    pending: BTreeSet<DeclarationId>,
    formation_deferred: bool,
}

impl std::fmt::Display for CallableClausePacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(callable-clause-packet (expression {}) (identities",
            self.expression
        )?;
        for identity in &self.identities {
            write!(f, " {identity}")?;
        }
        f.write_str(") (pending")?;
        for parameter in &self.pending {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (formation-deferred {}))", self.formation_deferred)
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
        self,
        checker: &mut Checker<'_, '_>,
        telescope: &[(DeclarationId, Sort, Term)],
        inputs: declaration_inputs::PreparedDeclarationInputs,
        span: Span,
    ) -> Result<Self::Output, Error> {
        let Self {
            owner,
            parameters,
            domains,
            prefix,
        } = self;
        let prepared = inputs.into_typed(telescope, span)?;
        let mut symbols = Vec::new();
        for value in &prefix {
            symbols.push(checker.copy_alias_type(value, span)?);
        }
        symbols.extend(prepared.formation.terms);
        let pending = prepared.formation.deferred;
        let mut operands = Vec::new();
        for (value, domain) in prefix.into_iter().zip(&domains) {
            operands.push(n::Expr {
                source: source(span),
                ty: checker.copy_alias_type(domain, span)?,
                kind: if *domain == p(P::Type) {
                    K::TypeValue(value)
                } else {
                    K::ComptimeValue(value)
                },
            });
        }
        let statements = prepared.statements;
        operands.extend(prepared.arguments);
        let formation = FormationInputs {
            terms: symbols
                .iter()
                .map(|value| checker.copy_alias_type(value, span))
                .collect::<Result<_, _>>()?,
            deferred: parameters
                .iter()
                .zip(&domains)
                .filter_map(|(id, domain)| {
                    (*domain != p(P::Type) || pending.contains(id)).then_some(*id)
                })
                .collect(),
            environment: BTreeMap::new(),
        };
        let formation_deferred = !formation.deferred.is_empty();
        checker.check_formation_application(owner, formation, span)?;
        // Routing completion is not execution completion. Known domains are
        // checked even in unused definitions; domains still depending on this
        // packet's producers retain their declaration-side admission obligation.
        // The VM, or the required-input consumer for a runtime callable, checks
        // the completed telescope before forming a callable value.
        for (operand, domain) in operands.iter().zip(&domains) {
            let awaits_input = checker
                .term_declaration_inputs(domain, span)?
                .iter()
                .any(|parameter| pending.contains(parameter));
            let domain = checker.instantiate_input_types(domain, &parameters, &symbols, span)?;
            if awaits_input {
                checker.check_pending_callable_domain(&operand.ty, &domain, span)?;
            } else {
                checker.require(&operand.ty, &domain, span)?;
            }
        }
        let ty = Term::Tuple(
            operands
                .iter()
                .map(|value| checker.copy_alias_type(&value.ty, span))
                .collect::<Result<_, _>>()?,
        );
        Ok(CallableClausePacket {
            expression: n::Expr {
                source: source(span),
                ty: checker.copy_alias_type(&ty, span)?,
                kind: K::Block {
                    statements,
                    result: Some(Box::new(n::Expr {
                        source: source(span),
                        ty,
                        kind: K::Tuple(operands),
                    })),
                },
            },
            identities: symbols,
            pending,
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

impl Checker<'_, '_> {
    /// A negative proof rejects an already known conflict. Failure to prove a
    /// conflict does not admit the value: the receiving invocation or required
    /// input consumer must check the actual completed domain before publication.
    fn check_pending_callable_domain(
        &self,
        actual: &Term,
        expected: &Term,
        span: Span,
    ) -> Result<(), Error> {
        if is_never(actual) {
            return Ok(());
        }
        let (actual_view, expected_view) = self.scoped_equality_operands(actual, expected, span)?;
        let outcome = self.context.prove(
            crate::ptc::Formula::equal(actual_view, expected_view).negate(),
            crate::ptc::Budget::default(),
        );
        match outcome {
            crate::ptc::Outcome::Entailed(_) => Err(error(
                span,
                format!("type mismatch: expected {expected}, found {actual}"),
            )),
            crate::ptc::Outcome::NotEntailed(_)
            | crate::ptc::Outcome::Unknown {
                reason: crate::ptc::UnknownReason::UnvalidatedModel,
                ..
            } => Ok(()),
            crate::ptc::Outcome::Unknown { reason, .. } => Err(error(
                span,
                format!(
                    "pending input domain observation failed ({reason:?}): expected {expected}, found {actual}"
                ),
            )),
            crate::ptc::Outcome::IllFormed(reason) => Err(error(
                span,
                format!("ill-formed pending input domain: {reason}"),
            )),
        }
    }

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
        let domains = self.callable_clause_domains(owner, &parameters, prefix, expr.span)?;
        let prefix_binders = self.input_binders(&parameters[..prefix.len()]);
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
        if value_parameters
            .iter()
            .any(|id| signature.mentions_rigid(*id))
        {
            return Ok(None);
        }
        if !allow_optional {
            arguments::require_function_value_parameters(sig.params, expr.span)?;
        }
        let packet =
            self.prepare_callable_clause_packet(expr, owner, args, prefix, &parameters, &domains)?;
        let signature =
            self.instantiate_input_types(&signature, &parameters, &packet.identities, expr.span)?;
        if packet
            .pending
            .iter()
            .any(|parameter| signature.mentions_rigid(*parameter))
        {
            return Err(error(
                expr.span,
                "comptime callable signature still depends on an invocation-time clause input",
            ));
        }
        if packet.pending.is_empty()
            && !packet.formation_deferred
            && matches!(&packet.expression.kind, K::Block { statements, .. } if statements.is_empty())
        {
            // Only an already checked static application can use the direct
            // instance and declaration-side default substitution. An observed
            // identity cannot erase producers or complete deferred contracts.
            return self
                .function_instance(expr, owner, packet.identities, expected, allow_optional)
                .map(Some);
        }
        let ty = self.callable_type(owner, signature);
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(Some(n::Expr {
            source: source(expr.span),
            ty,
            kind: K::ComptimeFunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                subject: Box::new(packet.expression),
            },
        }))
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
        let completion =
            CallableCompletion::retain(self, owner, parameters, domains, prefix, expr.span)?;
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
        self.complete_declaration_inputs(
            owner,
            args,
            expr.span,
            telescope,
            Some(defaults),
            completion,
        )
    }

    fn callable_clause_domains(
        &mut self,
        owner: OwnerId,
        parameters: &[DeclarationId],
        prefix: &[Term],
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        if prefix.len() > parameters.len() {
            return Err(error(span, "clause input prefix arity mismatch"));
        }
        let declared_domains = self.trait_domains(owner)?;
        let prefix_binders = self.input_binders(&parameters[..prefix.len()]);
        declared_domains
            .iter()
            .map(|domain| {
                domain
                    .instantiate_parameters_in_scope(
                        &prefix_binders,
                        prefix,
                        &self.call_input_frames(),
                    )
                    .map_err(|failure| error(span, failure.to_string()))
            })
            .collect()
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
        let domains = self.callable_clause_domains(owner, &parameters, prefix, expr.span)?;
        let prepared = self.prepare_required_root_with_live_inputs(
            expr,
            required::RootKind::CallableInputs,
            Vec::new(),
            false,
            &[],
            |checker| {
                checker
                    .prepare_callable_clause_packet(
                        expr,
                        owner,
                        args,
                        prefix,
                        &parameters,
                        &domains,
                    )
                    .map(|packet| packet.expression)
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
            expression,
            identities: vec![Term::Rigid {
                declaration: first,
                sort: Sort::Type,
            }],
            pending: [last, first].into_iter().collect(),
            formation_deferred: true,
        };
        assert_eq!(
            packet.to_string(),
            format!(
                "(callable-clause-packet (expression {producer}) (identities {}) (pending {first} {last}) (formation-deferred true))",
                packet.identities[0]
            )
        );
    }
}
