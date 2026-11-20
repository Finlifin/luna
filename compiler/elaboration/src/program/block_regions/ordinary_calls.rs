//! One source region checks the input telescope of an instantiated ordinary call.
//! Dependencies select which producers to inspect. The source routes determine
//! where those producers run, independently of the order of type observations.
use super::*;
use dependent_inputs::{CallSignature, CallSite, InputProducers, PreparedInputs, tasks};
use driver::{Poll, Request, Root};

pub(in crate::program) type InstanceFrame<'h> =
    declaration_inputs::preparation::Frame<'h, dynamic_instances::InstanceCompletion<'h>>;

pub(in crate::program) enum ExplicitInstance<'h> {
    Ready(n::Expr),
    Checking(Box<InstanceFrame<'h>>),
}

pub(in crate::program) enum CallPlan<'h> {
    Ready(n::Expr),
    Inputs(Box<CallInputs<'h>>),
}

impl CallPlan<'_> {
    pub(in crate::program) fn finish(
        self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<n::Expr, Error> {
        match self {
            Self::Ready(value) => Ok(value),
            Self::Inputs(frame) => checker.drive_input_consumer(*frame),
        }
    }
}

/// The ordinary inputs remain unchecked while the region forms the callee.
/// Finishing that child selects a call plan in this same caller.
pub(super) enum Invocation<'h> {
    AwaitCallee {
        source: Expr<'h>,
        args: &'h [Arg<'h>],
        destination: Destination,
    },
    Inputs(Box<CallInputs<'h>>),
}

impl std::fmt::Display for Invocation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AwaitCallee {
                source: expr,
                args,
                destination,
            } => write!(
                f,
                "(await-callee (source {} {}) (ordinary-inputs {}) {})",
                expr.hir_id,
                source(expr.span),
                args.len(),
                destination
            ),
            Self::Inputs(frame) => frame.fmt(f),
        }
    }
}

impl<'h> Root<'h> for Invocation<'h> {
    type Output = n::Expr;
    type Invocation = required::Invocation;

    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h>, n::Expr, required::Invocation>, Error> {
        match self {
            Self::Inputs(frame) => frame.advance(checker, returned),
            Self::AwaitCallee {
                source,
                args,
                destination,
            } => {
                let callee = returned.expect("callee completion").ordinary(source.span)?;
                let view = destination.ordinary_view().expect("ordinary call result");
                match checker.plan_prepared_call(
                    source,
                    callee,
                    args,
                    vec![],
                    view.expected,
                    view.partial,
                    CallUse::Invocation,
                )? {
                    CallPlan::Ready(value) => Ok(Poll::Complete(value)),
                    CallPlan::Inputs(frame) => {
                        *self = Self::Inputs(frame);
                        Ok(Poll::Continue)
                    }
                }
            }
        }
    }

    fn resume(
        &mut self,
        checker: &mut Checker<'_, '_>,
        reply: required::CompletedInvocation,
    ) -> Result<(), Error> {
        let Self::Inputs(frame) = self else {
            unreachable!("callee checks own their children")
        };
        frame.resume(checker, reply)
    }

    fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        if let Self::Inputs(frame) = self {
            frame.unwind(checker);
        }
    }
}

enum Phase {
    Observe(tasks::Frame),
    Supply(PreparedInputs),
    Finished,
}

struct Completion<'h> {
    site: CallSite<'h>,
    retained_callable: Term,
    parameter_bindings: Vec<n::LocalId>,
    expected: Option<Term>,
}

pub(in crate::program) struct CallInputs<'h> {
    completion: Option<Completion<'h>>,
    assigned: Vec<bool>,
    offset: usize,
    next: usize,
    waiting: Option<(usize, bool)>,
    arguments: Vec<n::CallArgument>,
    phase: Phase,
}

impl<'h> Root<'h> for CallInputs<'h> {
    type Output = n::Expr;
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
    ) -> Result<Poll<Request<'h>, n::Expr, Self::Invocation>, Error> {
        let site = &self.completion.as_ref().expect("active call").site;
        if let Phase::Observe(dependencies) = &mut self.phase {
            let mut prepared = match dependencies.advance(checker, site, returned)? {
                Poll::Invoke(work) => return Ok(Poll::Invoke(work)),
                Poll::Continue => return Ok(Poll::Continue),
                Poll::Child(request) => return Ok(Poll::Child(request)),
                Poll::Complete(prepared) => prepared,
            };
            if let Some(owner) = site.owner {
                let type_arguments = checker
                    .method_instantiation_inputs(&site.callee, site.source.span)?
                    .map(|(_, arguments)| arguments)
                    .unwrap_or_default();
                checker.check_ordinary_capabilities(
                    owner,
                    &type_arguments,
                    prepared.binding.observations(),
                    site.source.span,
                )?;
                checker.check_ordinary_type_requirements(
                    owner,
                    &type_arguments,
                    prepared.binding.observations(),
                    site.source.span,
                )?;
            }
            for slot in 0..self.offset {
                let receiver = prepared.producers.values[slot]
                    .take()
                    .expect("prepared receiver");
                self.arguments.push(n::CallArgument {
                    passing: Default::default(),
                    parameter: slot,
                    value: receiver,
                    is_default: false,
                });
            }
            self.phase = Phase::Supply(prepared);
        } else if let Some(returned) = returned {
            let (slot, caller_staged) = self.waiting.take().expect("requested call input");
            checker.in_staged_input = caller_staged;
            self.arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter: slot,
                value: returned.ordinary(site.source.span)?,
                is_default: false,
            });
        }

        let Phase::Supply(PreparedInputs {
            signature,
            producers: prepared,
            binding,
        }) = &mut self.phase
        else {
            unreachable!()
        };
        let signature = CallSignature::of(signature).expect("prepared callable signature");
        while let Some(input) = site.routes.get(self.next) {
            self.next += 1;
            let slot = self.offset + input.parameter;
            let parameter = &signature.parameters[slot];
            let domain = binding.substitute(&parameter.ty).map_err(|failure| {
                error(
                    site.source.span,
                    format!("dependent call input is not available: {failure}"),
                )
            })?;
            let domain = checker.callable_input_view(&domain, site.source.span)?;
            input.require_collection_type(checker, &domain, site.source.span)?;
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
            return Ok(Poll::Child(Request::routed(
                input.requested_source(&site.source),
                domain,
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
                    .map_err(|failure| error(site.source.span, failure.to_string()))?;
                value.rewrite_types(&mut |ty| {
                    checker.callable_argument_term_view(ty, site.source.span)
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
        let Phase::Supply(prepared) = std::mem::replace(&mut self.phase, Phase::Finished) else {
            unreachable!()
        };
        let completion = self.completion.take().expect("active call");
        completion
            .finish(
                checker,
                prepared,
                std::mem::take(&mut self.arguments),
                self.offset,
            )
            .map(Poll::Complete)
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

impl std::fmt::Display for CallInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(ordinary-call-inputs (next {})", self.next)?;
        if let Some(completion) = &self.completion {
            write!(f, " {}", completion.site)?;
        }
        match &self.phase {
            Phase::Observe(frame) => write!(f, " (observe {frame})")?,
            Phase::Supply(PreparedInputs {
                producers: prepared,
                binding,
                ..
            }) => write!(f, " (supply {binding} {prepared})")?,
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

impl Completion<'_> {
    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        prepared: PreparedInputs,
        mut arguments: Vec<n::CallArgument>,
        offset: usize,
    ) -> Result<n::Expr, Error> {
        let Self {
            site,
            retained_callable,
            parameter_bindings,
            expected,
        } = self;
        let expr = &site.source;
        let signature =
            CallSignature::of(&prepared.signature).expect("prepared callable signature");
        let call_binding = prepared.binding;
        let ty = if arguments
            .iter()
            .any(|argument| is_never(&argument.value.ty))
        {
            p(P::Never)
        } else {
            let result = call_binding.substitute(signature.result).map_err(|e| {
                error(
                    expr.span,
                    format!("dependent call result is not available: {e}"),
                )
            })?;
            checker.callable_input_view(&result, expr.span)?
        };
        if let Some(expected) = &expected {
            checker.require_expression(&ty, expected, expr.span)?;
        }
        // Retain the interface-origin signature for phase validation. The
        // selected projection view above checks inputs/results; IIR closure
        // resolves all retained annotations together without losing origin.
        checker.comptime_receiver_arguments(site.owner, &mut arguments);
        let mut callee = site.callee;
        callee.ty = retained_callable;
        let inputs = checker.call_environment_inputs(&callee, expr.span)?;
        let mut call = n::Expr {
            ty,
            source: source(expr.span),
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                env_arguments: Vec::new(),
                parameter_bindings,
            },
        };
        let environment =
            checker.prepare_call_environment(&mut call, inputs, offset, &site.routes, expr)?;
        let call = checker.freeze_staged_call_inputs(call, expr)?;
        environment.finish(checker, call, &site.routes, offset, expr)
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn plan_ordinary_call<'h>(
        &mut self,
        site: CallSite<'h>,
        callable: Term,
        retained_callable: Term,
        assigned: Vec<bool>,
        prefix: Vec<n::Expr>,
        parameter_bindings: Vec<n::LocalId>,
        expected: Option<&Term>,
    ) -> Result<CallPlan<'h>, Error> {
        let offset = prefix.len();
        let count = assigned.len() + offset;
        let mut prepared = InputProducers::new(count);
        for (slot, value) in prefix.into_iter().enumerate() {
            prepared.values[slot] = Some(value);
        }
        let dependencies = tasks::Frame::new(self, callable, &site, &assigned, offset, prepared)?;
        let expected = expected
            .map(|ty| self.copy_alias_type(ty, site.source.span))
            .transpose()?;
        Ok(CallPlan::Inputs(Box::new(CallInputs {
            completion: Some(Completion {
                site,
                retained_callable,
                parameter_bindings,
                expected,
            }),
            assigned,
            offset,
            next: 0,
            waiting: None,
            arguments: Vec::with_capacity(count),
            phase: Phase::Observe(dependencies),
        })))
    }
}
