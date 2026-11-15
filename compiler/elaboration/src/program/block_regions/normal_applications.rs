//! NF input preparation shares the expression region's child stack. Completion
//! retains the opaque head and source schedule; no argument is executed here.
use super::*;
use declaration_inputs::{Completion, InputChecking, PreparedDeclarationInputs, preparation};
use driver::{Request, Step};

struct Complete {
    owner: OwnerId,
    destination: Destination,
    preserve_effect: Option<bool>,
}

impl Completion for Complete {
    const CHECKING: InputChecking = InputChecking::Contextual;
    type Output = n::Expr;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        _: &[(DeclarationId, Sort, Term)],
        inputs: PreparedDeclarationInputs,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let ty = if inputs.diverges() {
            p(P::Never)
        } else {
            p(P::Type)
        };
        checker.check_formation_application(self.owner, inputs.formation, span)?;
        if let Some(expected) = self.destination.expected() {
            checker.require_expression(&ty, expected, span)?;
        }
        let value = n::Expr {
            source: source(span),
            ty,
            kind: K::NormalFormType {
                head: declaration(checker.package, &checker.package_identities, self.owner),
                arguments: inputs.arguments,
                plan: Some(inputs.plan),
            },
        };
        if self.preserve_effect == Some(false) {
            checker.run_computation(value, self.destination.expected(), span)
        } else {
            Ok(value)
        }
    }
}

impl std::fmt::Display for Complete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(normal-application-completion (owner {}) {} (effect-delivery {}))",
            self.owner,
            self.destination,
            match self.preserve_effect {
                None => "caller",
                Some(true) => "preserve",
                Some(false) => "consume",
            }
        )
    }
}

pub(super) struct Frame<'h> {
    span: Span,
    preparation: preparation::Frame<'h, Complete>,
}

impl<'h> Frame<'h> {
    pub(super) fn new<'a: 'h, 'arena: 'h>(
        checker: &mut Checker<'a, 'arena>,
        owner: OwnerId,
        inputs: &[Arg<'h>],
        span: Span,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        checker.ensure_signature(owner, span)?;
        checker.ensure_contract(owner, span)?;
        let parameters = checker.normal_parameters[&owner]
            .iter()
            .map(|(id, _, domain)| {
                Ok((
                    *id,
                    if *domain == p(P::Type) {
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    },
                    declaration_inputs::copy_input_domain(
                        domain,
                        &mut checker.required_steps,
                        span,
                    )?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let completion = Complete {
            owner,
            destination,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        };
        let preparation =
            preparation::Frame::begin(checker, owner, inputs, span, parameters, None, completion)?;
        Ok(Self { span, preparation })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h, 'h>, Error> {
        if let Some(value) = returned {
            let value = match value {
                CheckedBlock::Ordinary(value) => n::NormalFormArgument::Value(value),
                CheckedBlock::Contextual(value) => value,
            };
            self.preparation.accept(checker, Ok(value))?;
        }
        match self.preparation.poll(checker)? {
            preparation::Poll::Continue => Ok(Step::Continue),
            preparation::Poll::Complete(value) => {
                Ok(Step::Completed(CheckedBlock::Ordinary(value)))
            }
            preparation::Poll::Child(request) => {
                use declaration_inputs::request::Demand;
                let (input, demand) = request.into_parts();
                let destination = match demand {
                    Demand::Ordinary(expected) => Destination::ordinary(
                        checker,
                        OrdinaryTail {
                            expected: expected.as_ref(),
                            partial: None,
                            input: None,
                        },
                        input.span,
                    )?,
                    Demand::Input(context) => Destination::ordinary(
                        checker,
                        OrdinaryTail {
                            expected: None,
                            partial: None,
                            input: Some(&context),
                        },
                        input.span,
                    )?,
                    // The old request enters dependent_normal_argument at depth 0.
                    Demand::Contextual => Destination::ContextualInput,
                };
                Ok(Step::Child(Request::Expression(input, destination)))
            }
        }
    }

    pub(super) fn recover(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        // Collection probes own their failure recovery inside input preparation.
        self.preparation.reject(checker, failure)
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        self.preparation.leave(checker);
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(normal-application-frame (source {}) {})",
            source(self.span),
            self.preparation
        )
    }
}
