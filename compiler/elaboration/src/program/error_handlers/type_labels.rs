//! Error rows consume ordinary comptime Type values or collected Type values.
use super::*;
use std::sync::Arc;

pub(in crate::program) enum ErrorTypeRow {
    Labels(Vec<Term>),
    Deferred(Term),
}

fn has_open_comptime_call_input(term: &nir::ComptimeCall) -> bool {
    let mut stack: Vec<&Term> = term
        .type_arguments
        .iter()
        .chain(term.arguments.iter())
        .chain(std::iter::once(&term.ty))
        .collect();
    while let Some(node) = stack.pop() {
        match node {
            Term::Bound { .. }
            | Term::Rigid { .. }
            | Term::Hole { .. }
            | Term::AssociatedType { .. }
            | Term::AssociatedTypeCall(_)
            | Term::ComptimeCall(_)
            | Term::DeferredErrorResult { .. }
            | Term::RuntimeField { .. } => return true,
            _ => {
                let _ = node.visit_children(|child| {
                    stack.push(child);
                    Ok::<_, ()>(())
                });
            }
        }
    }
    false
}

impl Checker<'_, '_> {
    fn expr_symbolic_argument(&self, expr: &n::Expr) -> Option<Term> {
        match &expr.kind {
            K::TypeValue(term) | K::ComptimeValue(term) => Some(term.clone()),
            K::Local(id) => self
                .input_facts
                .parameters
                .runtime_identity(id)
                .map(|(term, _)| term.clone())
                .or_else(|| self.input_facts.types.get(id).cloned())
                .or_else(|| {
                    self.input_facts
                        .values
                        .get(id)
                        .map(|(term, _)| term.clone())
                }),
            K::Unit => Some(Term::Unit),
            K::Literal(n::Literal::Bool(value)) => Some(Term::Bool(*value)),
            K::Literal(n::Literal::Char(value)) => Some(Term::Char(*value)),
            K::Literal(n::Literal::String(value)) => Some(Term::String(value.clone().into())),
            K::Literal(n::Literal::Symbol(value)) => Some(Term::Symbol(value.clone().into())),
            K::Literal(n::Literal::SignedInteger(value)) => {
                Some(Term::Integer(comptime_runtime::Integer::from_i128(*value)))
            }
            K::Literal(n::Literal::UnsignedInteger(value)) => {
                Some(Term::Integer(comptime_runtime::Integer::from_u128(*value)))
            }
            K::Literal(n::Literal::Float16(value)) => Some(Term::Float16(*value)),
            K::Literal(n::Literal::BFloat16(value)) => Some(Term::BFloat16(*value)),
            K::Literal(n::Literal::Float32(value)) => Some(Term::Float32(*value)),
            K::Literal(n::Literal::Float64(value)) => Some(Term::Float64(*value)),
            _ => None,
        }
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn error_type_row(
        &mut self,
        source: &Expr<'_>,
    ) -> Result<ErrorTypeRow, Error> {
        // Literal rows also work before a collection protocol is installed,
        // and may contain the current signature's symbolic Type parameters.
        if let H::List(items) = &source.kind {
            let labels = items
                .iter()
                .map(|item| self.type_expr(item))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(ErrorTypeRow::Labels(labels));
        }
        let checked = self.with_staged_input(true, |this| this.expr(source, None))?;
        if checked.ty == p(P::Type) {
            // Preserve normal-form identity: observing a Type in a row must
            // not turn a head application into invocation of its body.
            return Ok(ErrorTypeRow::Labels(vec![self.type_expr(source)?]));
        }
        if self.list_element_type(&checked.ty) != Some(&p(P::Type)) {
            return Err(error(
                source.span,
                "error row requires Type or a comptime list of Type values",
            ));
        }
        if let Some(call) = self.open_comptime_error_call(&checked)? {
            if let Some(labels) = self.simplify_comptime_call_under_context(&call, source.span)? {
                return Ok(ErrorTypeRow::Labels(labels));
            }
            return Ok(ErrorTypeRow::Deferred(Term::ComptimeCall(Box::new(call))));
        }
        let domain = checked.ty.clone();
        let (value, _) = self.required_normal_input(source, checked, Vec::new())?;
        let comptime_interpretor::Value::List { ty, values } = value else {
            return Err(error(
                source.span,
                "error row did not produce a comptime list",
            ));
        };
        if ty != domain {
            return Err(error(
                source.span,
                "error row list domain changed during invocation",
            ));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(values.len() as u64)
            .ok_or_else(|| error(source.span, "error row label budget exhausted"))?;
        let labels = values
            .into_iter()
            .map(|value| match value {
                comptime_interpretor::Value::Type(ty) if ty.sort() == Sort::Type => Ok(ty),
                _ => Err(error(
                    source.span,
                    "error row list contains a non-Type value",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ErrorTypeRow::Labels(labels))
    }

    fn open_comptime_error_call(
        &self,
        checked: &n::Expr,
    ) -> Result<Option<nir::ComptimeCall>, Error> {
        let K::Call {
            callee,
            arguments,
            env_arguments,
            ..
        } = &checked.kind
        else {
            return Ok(None);
        };
        if !env_arguments.is_empty()
            || !callee.ty.is_comptime_callable()
            || !callee.ty.is_pure_callable()
        {
            return Ok(None);
        }
        let (declaration, type_arguments) = match &callee.kind {
            K::Function(declaration) => (*declaration, Vec::new()),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            _ => return Ok(None),
        };
        let mut call_arguments = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let Some(value) = self.expr_symbolic_argument(&argument.value) else {
                return Ok(None);
            };
            call_arguments.push(value);
        }
        let call = nir::ComptimeCall {
            declaration,
            type_arguments,
            arguments: call_arguments,
            ty: checked.ty.clone(),
        };
        if has_open_comptime_call_input(&call) {
            Ok(Some(call))
        } else {
            Ok(None)
        }
    }

    pub(in crate::program) fn simplify_comptime_call_under_context(
        &mut self,
        call: &nir::ComptimeCall,
        span: Span,
    ) -> Result<Option<Vec<Term>>, Error> {
        if !self.interfaces_ready {
            return Ok(None);
        }
        let Some(owner) = self.source_declarations.owner(call.declaration) else {
            return Ok(None);
        };
        let Some(ItemKind::Fn(sig, body_id)) = self.package.item(owner).map(|i| &i.kind) else {
            return Ok(None);
        };
        if !sig.modifiers.is_comptime || !sig.modifiers.is_pure {
            return Ok(None);
        }
        let Some(body) = self.package.body(*body_id) else {
            return Ok(None);
        };
        let saved_locals = self.locals.clone();
        let saved_facts = self.input_facts.clone();
        let saved_aliases = self.alias_substitution.clone();
        let saved_owner = self.signature_owner.replace(owner);
        let saved_code_body = std::mem::replace(&mut self.in_code_body, false);
        let saved_required = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_staged = std::mem::replace(&mut self.in_staged_input, true);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_env_parameters = std::mem::take(&mut self.active_env_parameters);
        let result = (|| -> Result<Option<Vec<Term>>, Error> {
            let type_params = self
                .type_parameters
                .get(&owner)
                .cloned()
                .unwrap_or_default();
            for (binder, argument) in type_params.iter().zip(&call.type_arguments) {
                self.alias_substitution
                    .insert(*binder, Arc::new(argument.clone()));
            }
            for (parameter, argument) in sig.params.iter().zip(&call.arguments) {
                let binding = local(parameter.hir_id);
                if argument.sort() == Sort::Type {
                    self.input_facts.types.insert(binding, argument.clone());
                    self.locals.insert(binding, p(P::Type));
                } else if argument.sort() == Sort::Runtime {
                    let domain = Term::Lift(Box::new(Term::AnyType));
                    self.input_facts.parameters.bind_runtime_value(
                        binding,
                        argument.clone(),
                        domain,
                    );
                    self.locals.insert(binding, Term::AnyType);
                } else {
                    self.input_facts
                        .values
                        .insert(binding, (argument.clone(), Term::AnyType));
                }
            }
            let mut current = body.value;
            for _ in 0..32 {
                loop {
                    match current.kind {
                        H::Block(block) if block.stmts.is_empty() => {
                            if let Some(tail) = block.expr {
                                current = tail;
                                continue;
                            }
                        }
                        H::InlineControl(inner) => {
                            current = inner;
                            continue;
                        }
                        _ => {}
                    }
                    break;
                }
                let H::If(condition, yes, Some(no)) = current.kind else {
                    break;
                };
                if !(self.extracted_refinement(condition) || self.is_type_assumption(condition)) {
                    break;
                }
                let predicate = self.type_predicate(condition)?;
                let fact = crate::ptc::Formula::from_predicate(&predicate);
                if matches!(
                    self.context
                        .prove(fact.clone(), crate::ptc::Budget::default()),
                    crate::ptc::Outcome::Entailed(_)
                ) {
                    if yes.stmts.is_empty()
                        && let Some(tail) = yes.expr
                    {
                        current = tail;
                        continue;
                    }
                    return Ok(None);
                }
                if matches!(
                    self.context
                        .prove(fact.negate(), crate::ptc::Budget::default()),
                    crate::ptc::Outcome::Entailed(_)
                ) {
                    current = no;
                    continue;
                }
                return Ok(None);
            }
            if let H::List(items) = &current.kind {
                let labels = items
                    .iter()
                    .map(|item| self.type_expr(item))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(Some(labels));
            }
            Ok(None)
        })();
        self.active_env_parameters = saved_env_parameters;
        self.captures = saved_captures;
        self.in_staged_input = saved_staged;
        self.in_required_comptime = saved_required;
        self.in_code_body = saved_code_body;
        self.signature_owner = saved_owner;
        self.locals = saved_locals;
        self.input_facts = saved_facts;
        self.alias_substitution = saved_aliases;
        let _ = span;
        result
    }

    pub(in crate::program) fn simplify_deferred_error_result(
        &mut self,
        ty: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let Term::DeferredErrorResult { errors, result } = ty else {
            return Ok(ty.clone());
        };
        if let Term::ComptimeCall(call) = errors.as_ref()
            && let Some(labels) = self.simplify_comptime_call_under_context(call, span)?
        {
            return Ok(Term::error_result(labels, (**result).clone()));
        }
        Ok(ty.clone())
    }
}
