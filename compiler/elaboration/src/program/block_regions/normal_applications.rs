//! NF input preparation shares the expression region's child stack. Completion
//! retains the opaque head and source schedule; no argument is executed here.
use super::*;
use declaration_inputs::{Completion, InputChecking, PreparedDeclarationInputs, preparation};

pub(super) struct Complete {
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
        checker.check_formation_application(
            self.owner,
            inputs.observations.into_formation(),
            span,
        )?;
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

pub(super) type Frame<'h> = preparation::Frame<'h, Complete>;

impl<'h> Frame<'h> {
    pub(super) fn begin_normal_application<'a: 'h, 'arena: 'h>(
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
        preparation::Frame::begin(
            checker,
            owner,
            inputs,
            span,
            parameters,
            declaration_inputs::KnownInputs::default(),
            None,
            completion,
        )
    }
}
