//! One source region checks the input telescope of an instantiated ordinary call.
//! Dependencies select which producers to inspect. The source routes determine
//! where those producers run, independently of the order of type observations.
use super::*;
use dependent_inputs::{CallSignature, PreparedInputs, tasks};
use driver::{Poll, Request, Root};

type Inputs = (Vec<n::CallArgument>, nir::FunctionCallBinding);

enum Phase<'b, 'h> {
    Observe(tasks::Frame<'b, 'h>),
    Supply(PreparedInputs, nir::FunctionCallBinding),
    Finished,
}

struct CallInputs<'b, 'h> {
    signature: CallSignature<'b>,
    callee: &'b n::Expr,
    owner: Option<OwnerId>,
    routes: &'b [arguments::RoutedArgument<'h>],
    source: &'b Expr<'h>,
    assigned: &'b [bool],
    offset: usize,
    next: usize,
    waiting: Option<(usize, bool)>,
    arguments: Vec<n::CallArgument>,
    phase: Phase<'b, 'h>,
}

impl CallInputs<'_, '_> {
    fn supply(&mut self, parameter: usize, value: n::Expr, is_default: bool) {
        self.arguments.push(n::CallArgument {
            passing: Default::default(),
            parameter,
            value,
            is_default,
        });
    }
}

impl<'b, 'h: 'b> Root<'b> for CallInputs<'b, 'h> {
    type Output = Inputs;
    type Invocation = required::Invocation;

    fn resume(
        &mut self,
        checker: &mut Checker<'_, '_>,
        reply: required::CompletedInvocation,
    ) -> Result<(), Error> {
        let Phase::Observe(dependencies) = &mut self.phase else {
            unreachable!("only dependency observation requests invocation")
        };
        dependencies.accept_required(checker, reply)
    }

    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'b>, Inputs, Self::Invocation>, Error> {
        if let Phase::Observe(dependencies) = &mut self.phase {
            let mut prepared = match dependencies.advance(checker, returned)? {
                Poll::Invoke(work) => return Ok(Poll::Invoke(work)),
                Poll::Continue => return Ok(Poll::Continue),
                Poll::Child(request) => return Ok(Poll::Child(request)),
                Poll::Complete(prepared) => prepared,
            };
            let binding = checker.prepare_call_binding(
                self.signature.term,
                &prepared.symbols,
                self.source.span,
            )?;
            if let Some(owner) = self.owner {
                let type_arguments = checker
                    .method_instantiation_inputs(self.callee, self.source.span)?
                    .map(|(_, arguments)| arguments)
                    .unwrap_or_default();
                checker.check_ordinary_capabilities(
                    owner,
                    &type_arguments,
                    &prepared.symbols,
                    self.source.span,
                )?;
                checker.check_ordinary_type_requirements(
                    owner,
                    &type_arguments,
                    &prepared.symbols,
                    self.source.span,
                )?;
            }
            for slot in 0..self.offset {
                let receiver = prepared.values[slot].take().expect("prepared receiver");
                self.supply(slot, receiver, false);
            }
            self.phase = Phase::Supply(prepared, binding);
        } else if let Some(returned) = returned {
            let (slot, caller_staged) = self.waiting.take().expect("requested call input");
            checker.in_staged_input = caller_staged;
            self.supply(slot, returned.ordinary(self.source.span)?, false);
        }

        let Phase::Supply(prepared, binding) = &mut self.phase else {
            unreachable!()
        };
        while let Some(input) = self.routes.get(self.next) {
            self.next += 1;
            let slot = self.offset + input.parameter;
            let parameter = &self.signature.parameters[slot];
            let domain = binding.substitute(&parameter.ty).map_err(|failure| {
                error(
                    self.source.span,
                    format!("dependent call input is not available: {failure}"),
                )
            })?;
            let domain = checker.callable_input_view(&domain, self.source.span)?;
            input.require_collection_type(checker, &domain, self.source.span)?;
            if let Some(value) = prepared.values[slot].take() {
                self.arguments.push(n::CallArgument {
                    passing: Default::default(),
                    parameter: slot,
                    value,
                    is_default: false,
                });
                continue;
            }
            self.waiting = Some((slot, checker.in_staged_input));
            checker.in_staged_input |= parameter.sort != Sort::Runtime;
            return Ok(Poll::Child(Request::Expression(
                input.expression_view(self.source),
                Destination::from_expected(Some(domain)),
            )));
        }
        for (index, assigned) in self.assigned.iter().enumerate() {
            if *assigned {
                continue;
            }
            let parameter = index + self.offset;
            let value = if let Some(value) = prepared.values[parameter].take() {
                value
            } else {
                let mut value = prepared.defaults[parameter]
                    .take()
                    .expect("prepared default");
                value
                    .rewrite_types(&mut |ty| binding.substitute(ty))
                    .map_err(|failure| error(self.source.span, failure.to_string()))?;
                value.rewrite_types(&mut |ty| {
                    checker.callable_argument_term_view(ty, self.source.span)
                })?;
                value
            };
            self.arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter,
                value,
                is_default: true,
            });
        }
        let Phase::Supply(_, binding) = std::mem::replace(&mut self.phase, Phase::Finished) else {
            unreachable!()
        };
        Ok(Poll::Complete((
            std::mem::take(&mut self.arguments),
            binding,
        )))
    }

    fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        if let Phase::Observe(frame) = &mut self.phase {
            frame.unwind(checker);
        }
        if let Some((_, caller_staged)) = self.waiting.take() {
            checker.in_staged_input = caller_staged;
        }
    }
}

impl std::fmt::Display for CallInputs<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(ordinary-call-inputs (source {}) (next {})",
            source(self.source.span),
            self.next
        )?;
        match &self.phase {
            Phase::Observe(frame) => write!(f, " (observe {frame})")?,
            Phase::Supply(prepared, binding) => write!(f, " (supply {binding} {prepared})")?,
            Phase::Finished => f.write_str(" (complete)")?,
        }
        if let Some((slot, caller_staged)) = self.waiting {
            write!(f, " (waiting {slot} (caller-staged {caller_staged}))")?;
        }
        f.write_str(" (arguments")?;
        for argument in &self.arguments {
            write!(
                f,
                " (slot {} (default {}) {})",
                argument.parameter, argument.is_default, argument.value
            )?;
        }
        f.write_str("))")
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn check_ordinary_call_inputs(
        &mut self,
        signature: CallSignature<'_>,
        callee: &n::Expr,
        owner: Option<OwnerId>,
        routes: &[arguments::RoutedArgument<'_>],
        assigned: &[bool],
        prefix: Vec<n::Expr>,
        expr: &Expr<'_>,
    ) -> Result<Inputs, Error> {
        let offset = prefix.len();
        let mut prepared = PreparedInputs::new(signature.parameters.len());
        for (slot, value) in prefix.into_iter().enumerate() {
            prepared.values[slot] = Some(value);
        }
        let demanded = self.call_input_demands(
            signature,
            callee,
            owner,
            assigned,
            offset,
            expr,
            &mut prepared.defaults,
        )?;
        let dependencies = tasks::Frame::new(
            self, demanded, signature, callee, owner, routes, offset, expr, prepared,
        )?;
        self.drive_input_consumer(CallInputs {
            signature,
            callee,
            owner,
            routes,
            assigned,
            source: expr,
            offset,
            next: 0,
            waiting: None,
            arguments: Vec::with_capacity(signature.parameters.len()),
            phase: Phase::Observe(dependencies),
        })
    }
}
