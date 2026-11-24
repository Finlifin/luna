//! NF input preparation shares the expression region's child stack. Completion
//! retains the opaque head and source schedule; no argument is executed here.
use super::*;
use declaration_inputs::{Completion, InputChecking, PreparedDeclarationInputs, preparation};

pub(super) struct Complete {
    head: OwnerId,
    destination: Destination,
    preserve_effect: Option<bool>,
}

impl Completion for Complete {
    const CHECKING: InputChecking = InputChecking::Contextual;

    type Output = n::Expr;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        telescope: declaration_inputs::PreparedTelescope,
        inputs: PreparedDeclarationInputs,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let result_template = checker
            .signatures
            .get(&self.head)
            .or_else(|| checker.interface_signatures.get(&self.head))
            .and_then(|ty| match ty {
                Term::Function { result, .. } => Some(result.as_ref()),
                _ => None,
            })
            .cloned()
            .ok_or_else(|| error(span, "normal-form callable is missing result signature"))?;
        let diverges = inputs.diverges();
        let (ty, formation) = checker.instantiate_normal_result(
            &telescope.local,
            &result_template,
            inputs.observations,
            diverges,
            span,
        )?;
        checker.check_formation_application(self.head, formation, span)?;
        if let Some(expected) = self.destination.expected() {
            checker.require_expression(&ty, expected, span)?;
        }
        let value = n::Expr {
            source: source(span),
            ty,
            kind: K::NormalFormType {
                head: declaration(checker.package, &checker.package_identities, self.head),
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
            "(normal-application-completion (head {}) {} (effect-delivery {}))",
            self.head,
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
        application: normal::Application,
        inputs: &[Arg<'h>],
        span: Span,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        let owner = application.head;
        checker.ensure_signature(owner, span)?;
        checker.ensure_contract(owner, span)?;
        let parameters = checker.normal_input_parameters(owner, span)?;
        let completion = Complete {
            head: owner,
            destination,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        };
        match application.interface {
            normal::Supply::Declaration(owner) => {
                let telescope = declaration_inputs::Telescope::new(
                    parameters,
                    &mut checker.required_steps,
                    span,
                )?;
                preparation::Frame::begin(
                    checker,
                    owner,
                    inputs,
                    span,
                    telescope,
                    declaration_inputs::KnownInputs::default(),
                    None,
                    completion,
                )
            }
            normal::Supply::Callable { collector } => preparation::Frame::callable(
                checker,
                inputs,
                span,
                parameters,
                collector.as_ref(),
                completion,
            ),
        }
    }
}
