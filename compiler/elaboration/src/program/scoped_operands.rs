//! Bind a computed Type in the consumer that can check its result parametrically.
//! The producer is an ordinary argument of the generated comptime function.
use super::*;
use std::sync::Arc;

type Inputs = BTreeMap<usize, nir::DeclarationParameter>;

#[derive(Clone)]
pub(super) struct Scope {
    site: hir::HirId,
    inputs: Option<Arc<Inputs>>,
}

/// The checking region owns the temporary Type identity until it has either
/// abstracted every use or rejected the escaping consumer.
#[derive(Default)]
pub(super) enum Boundary {
    #[default]
    Idle,
    Selector {
        enclosing: Option<Scope>,
    },
    Consumer {
        inputs: Arc<Inputs>,
    },
}

impl Boundary {
    pub(super) fn enter(&mut self, checker: &mut Checker<'_, '_>, site: hir::HirId) {
        let enclosing = checker.operand_scope.replace(Scope { site, inputs: None });
        *self = Self::Selector { enclosing };
    }

    pub(super) fn leave(&mut self, checker: &mut Checker<'_, '_>) {
        if let Self::Selector { enclosing } = self {
            let selected = std::mem::replace(&mut checker.operand_scope, enclosing.take());
            *self = match selected.and_then(|scope| scope.inputs) {
                Some(inputs) => Self::Consumer { inputs },
                None => Self::Idle,
            };
        }
    }

    pub(super) fn close(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        match std::mem::take(self) {
            Self::Consumer { inputs } => checker.close_operand_consumer(value, &inputs, span),
            Self::Idle => Ok(value),
            Self::Selector { .. } => unreachable!("selector must complete before its consumer"),
        }
    }
}

impl std::fmt::Display for Boundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => f.write_str("(operand-scope idle)"),
            Self::Selector { .. } => f.write_str("(operand-scope selector)"),
            Self::Consumer { inputs } => {
                f.write_str("(operand-scope")?;
                for (slot, binder) in inputs.iter() {
                    write!(f, " (input (slot {slot}) {binder})")?;
                }
                f.write_str(")")
            }
        }
    }
}

struct CallPrefix {
    domains: Vec<Parameter>,
    bindings: Vec<n::LocalId>,
    suppliers: Vec<n::CallArgument>,
}

fn needs_frame(span: Span, reason: &str) -> Error {
    context_error(
        span,
        format!("dependent call input needs its original comptime invocation: {reason}"),
    )
}

impl Checker<'_, '_> {
    pub(super) fn bind_scoped_operand(
        &mut self,
        site: hir::HirId,
        slot: usize,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some(scope) = self
            .operand_scope
            .as_ref()
            .filter(|scope| scope.site == site)
        else {
            return Ok(None);
        };
        let binder = match scope.inputs.as_ref().and_then(|inputs| inputs.get(&slot)) {
            Some(binder) => *binder,
            None => {
                let binder = self.fresh_type_binder(site.owner, span)?;
                let scope = self.operand_scope.as_mut().unwrap();
                Arc::make_mut(scope.inputs.get_or_insert_with(Arc::default)).insert(slot, binder);
                binder
            }
        };
        Ok(Some(Term::Rigid {
            declaration: binder.declaration,
            sort: binder.sort,
        }))
    }

    fn close_operand_consumer(
        &mut self,
        mut body: n::Expr,
        inputs: &Inputs,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let K::Match { value, arms } = &mut body.kind else {
            return Err(needs_frame(span, "expected the original match"));
        };
        let identities: BTreeSet<_> = inputs.values().map(|binder| binder.declaration).collect();
        let prefix = self.take_operand_prefix(value, inputs, &identities, span)?;
        let outer_result =
            declaration_inputs::copy_input_domain(&body.ty, &mut self.required_steps, span)?;
        if !self
            .term_declaration_inputs(&outer_result, span)?
            .is_disjoint(&identities)
        {
            return Err(context_error(
                span,
                "dependent match result would escape its operand scope",
            ));
        }
        let caller_arms = self.detach_match_arms(arms, &identities, span)?;
        if caller_arms.is_some() {
            body.ty = p(P::Integer);
        }
        let free = self
            .with_call_type_fuel(|_, fuel| nir::free_bindings::collect(&body, fuel))
            .map_err(|failure| error(span, failure.to_string()))?;
        if free
            .iter()
            .any(|id| prefix.bindings.binary_search(id).is_err())
        {
            return Err(needs_frame(span, "match consumer reads an enclosing local"));
        }
        if !self.checked_substitution_inputs(&body, span)?.is_empty() {
            return Err(needs_frame(
                span,
                "match consumer reads an enclosing parameter scope",
            ));
        }
        if !self
            .checked_declaration_inputs(&body, span)?
            .is_subset(&identities)
        {
            return Err(needs_frame(
                span,
                "match consumer reads another declaration input",
            ));
        }
        self.with_call_type_fuel(|_, fuel| {
            nir::visit::expression(&body, fuel, |expr, _| {
                if matches!(
                    expr.kind,
                    K::Return(_) | K::Continue(_) | K::Break(_) | K::EffectResume(_)
                ) {
                    return Err("dependent match contains an enclosing control transfer".into());
                }
                Ok(())
            })
        })
        .map_err(|reason| context_error(span, reason))?;
        let result =
            declaration_inputs::copy_input_domain(&body.ty, &mut self.required_steps, span)?;
        nir::copy_budget::charge_type_rewrite(&body, &mut self.required_steps)
            .map_err(|limit| input_observation::structural_limit(limit, span))?;
        self.with_call_type_fuel(|_, fuel| {
            let frame =
                nir::function_parameters::DeclarationFrame::new(inputs.values().copied(), fuel)?;
            body.rewrite_types(&mut |ty| frame.abstract_term(ty, fuel))
        })
        .map_err(|reason| error(span, reason.to_string()))?;

        let call = self.publish_operand_consumer(body, prefix, result, span)?;
        let origin = call.source;
        Ok(match caller_arms {
            Some(arms) => n::Expr {
                source: origin,
                ty: outer_result,
                kind: K::Match {
                    value: Box::new(call),
                    arms,
                },
            },
            None => call,
        })
    }

    fn take_operand_prefix(
        &mut self,
        selector: &mut n::Expr,
        inputs: &Inputs,
        identities: &BTreeSet<DeclarationId>,
        span: Span,
    ) -> Result<CallPrefix, Error> {
        let K::Call {
            callee,
            arguments,
            env_arguments,
            ..
        } = &mut selector.kind
        else {
            return Err(needs_frame(span, "expected the original selector call"));
        };
        if !env_arguments.is_empty() {
            return Err(needs_frame(
                span,
                "selector requires its original callee environment",
            ));
        }
        // The helper signature binds identities in formal-slot order. Its call
        // below supplies their producers in source order, including named calls.
        let mut positions = BTreeMap::new();
        let mut bindings = Vec::new();
        let mut domains = Vec::new();
        for (&slot, binder) in inputs {
            positions.insert(slot, bindings.len());
            bindings.push(self.spread_binding(span)?);
            domains.push(Parameter {
                sort: binder.sort,
                ty: p(P::Type),
            });
        }
        // Forming a function instance or a closure is itself an evaluation.
        // Receive that value before the explicit arguments, in the caller.
        let target = self.spread_binding(span)?;
        let target_type =
            declaration_inputs::copy_input_domain(&callee.ty, &mut self.required_steps, span)?;
        let read = n::Expr {
            source: callee.source,
            ty: declaration_inputs::copy_input_domain(
                &target_type,
                &mut self.required_steps,
                span,
            )?,
            kind: K::Local(target),
        };
        let mut suppliers = vec![n::CallArgument {
            parameter: bindings.len(),
            value: std::mem::replace(callee.as_mut(), read),
            passing: n::ArgumentPassing::Value,
            is_default: false,
        }];
        bindings.push(target);
        domains.push(Parameter {
            sort: Sort::Runtime,
            ty: target_type,
        });
        for argument in arguments {
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            if argument.is_default || argument.passing != n::ArgumentPassing::Value {
                break;
            }
            let (position, binding) = if let Some(position) = positions.remove(&argument.parameter)
            {
                if !self
                    .checked_declaration_inputs(&argument.value, span)?
                    .is_disjoint(identities)
                {
                    return Err(needs_frame(
                        span,
                        "type operand uses another pending identity",
                    ));
                }
                (position, bindings[position])
            } else {
                // Computations with dependent annotations need the inner
                // telescope. Leave them and all following expressions there.
                let domain = &argument.value.ty;
                if !self.term_declaration_inputs(domain, span)?.is_empty()
                    || !self.term_substitution_inputs(domain, span)?.is_empty()
                    || !self
                        .checked_declaration_inputs(&argument.value, span)?
                        .is_disjoint(identities)
                {
                    break;
                }
                let binding = self.spread_binding(span)?;
                let position = bindings.len();
                bindings.push(binding);
                domains.push(Parameter {
                    sort: Sort::Runtime,
                    ty: declaration_inputs::copy_input_domain(
                        domain,
                        &mut self.required_steps,
                        span,
                    )?,
                });
                (position, binding)
            };
            let local = n::Expr {
                source: argument.value.source,
                ty: declaration_inputs::copy_input_domain(
                    &argument.value.ty,
                    &mut self.required_steps,
                    span,
                )?,
                kind: K::Local(binding),
            };
            suppliers.push(n::CallArgument {
                parameter: position,
                value: std::mem::replace(&mut argument.value, local),
                passing: n::ArgumentPassing::Value,
                is_default: false,
            });
        }
        if !positions.is_empty() {
            return Err(needs_frame(
                span,
                "operand depends on a preceding evaluation in the caller",
            ));
        }
        Ok(CallPrefix {
            domains,
            bindings,
            suppliers,
        })
    }

    fn publish_operand_consumer(
        &mut self,
        body: n::Expr,
        prefix: CallPrefix,
        result: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let CallPrefix {
            domains,
            bindings,
            suppliers,
        } = prefix;
        let declaration = self.generated_functions.fresh_identity(span)?;
        let signature = Term::Function {
            parameters: domains,
            result: Box::new(
                result
                    .abstract_parameters(&[])
                    .map_err(|reason| error(span, reason.to_string()))?,
            ),
        };
        let origin = body.source;
        let callable =
            declaration_inputs::copy_input_domain(&signature, &mut self.required_steps, span)?
                .with_comptime_phase(true);
        self.generated_functions.publish(n::Function {
            interface: n::FunctionDeclaration {
                declaration,
                signature,
                source: origin,
                phase: n::FunctionPhase::Comptime,
                name: format!("<scoped-operand.{}>", declaration.local),
                parameters: bindings.clone(),
                defaults: vec![],
                type_parameters: vec![],
                inline: false,
                pure: false,
                is_unsafe: false,
                env_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                entry_snapshots: vec![],
                result_guarantees: vec![],
            },
            body,
            runtime_deliveries: Default::default(),
        });
        Ok(n::Expr {
            source: origin,
            ty: result,
            kind: K::Call {
                callee: Box::new(n::Expr {
                    source: origin,
                    ty: callable,
                    kind: K::Function(declaration),
                }),
                arguments: suppliers,
                env_arguments: vec![],
                parameter_bindings: bindings,
            },
        })
    }

    /// Branches that do not consume pattern bindings stay in the caller. In
    /// particular, their returns, outer locals and writes keep their owner.
    fn detach_match_arms(
        &mut self,
        arms: &mut [n::MatchArm],
        identities: &BTreeSet<DeclarationId>,
        span: Span,
    ) -> Result<Option<Vec<n::MatchArm>>, Error> {
        for arm in arms.iter() {
            if !self
                .checked_declaration_inputs(&arm.body, span)?
                .is_disjoint(identities)
            {
                return Ok(None);
            }
            let uses_pattern = self
                .with_call_type_fuel(|_, fuel| {
                    let free = nir::free_bindings::collect(&arm.body, fuel)?;
                    if free.is_empty() {
                        return Ok(false);
                    }
                    let count = free.len();
                    let mut free: BTreeMap<_, _> = free.into_iter().map(|id| (id, ())).collect();
                    nir::free_bindings::remove_pattern_bindings(&arm.pattern, &mut free, fuel)?;
                    Ok::<_, nir::visit::Error>(free.len() != count)
                })
                .map_err(|failure| error(span, failure.to_string()))?;
            if uses_pattern {
                return Ok(None);
            }
        }
        let mut caller = Vec::with_capacity(arms.len());
        for (index, arm) in arms.iter_mut().enumerate() {
            let literal = n::Literal::Integer(index.to_string());
            let tag = n::Expr {
                source: arm.body.source,
                ty: p(P::Integer),
                kind: K::Literal(literal.clone()),
            };
            caller.push(n::MatchArm {
                pattern: n::Pattern::Literal(literal),
                guard: None,
                body: std::mem::replace(&mut arm.body, tag),
            });
        }
        Ok(Some(caller))
    }
}
