//! A preparation owns its consumer as well as its input-checking progress.
//! Completion runs only after the caller's lexical environment is restored.
use super::*;

pub(in crate::program) trait Completion: std::fmt::Display {
    const CHECKING: InputChecking;
    type Output;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        telescope: PreparedTelescope,
        inputs: PreparedDeclarationInputs,
        span: Span,
    ) -> Result<Self::Output, Error>;
}

pub(in crate::program) struct Contextual;
pub(super) struct Typed;

impl Completion for Contextual {
    const CHECKING: InputChecking = InputChecking::Contextual;
    type Output = PreparedDeclarationInputs;

    fn finish(
        self,
        _: &mut Checker<'_, '_>,
        _: PreparedTelescope,
        inputs: PreparedDeclarationInputs,
        _: Span,
    ) -> Result<Self::Output, Error> {
        Ok(inputs)
    }
}

impl Completion for Typed {
    const CHECKING: InputChecking = InputChecking::Typed;
    type Output = CheckedDeclarationInputs;

    fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        _: PreparedTelescope,
        inputs: PreparedDeclarationInputs,
        span: Span,
    ) -> Result<Self::Output, Error> {
        inputs.into_typed(&mut checker.required_steps, span)
    }
}

impl std::fmt::Display for Contextual {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(contextual-input-completion)")
    }
}

impl std::fmt::Display for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(typed-input-completion)")
    }
}
