//! Each checking region retains the consumer of its final result.
use super::*;

/// Expression children always return CheckedBlock. The root owns the consumer
/// that can turn those children into a different result in the original scope.
pub(in crate::program::block_regions) trait Root<'b, 'h>:
    std::fmt::Display
{
    type Output;
    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'b, 'h>, Self::Output>, Error>;
    fn recover(&mut self, _: &mut Checker<'_, '_>, failure: Error) -> Result<(), Error> {
        Err(failure)
    }
    fn unwind(&mut self, checker: &mut Checker<'_, '_>);
}

pub(super) struct ExpressionResult;

impl std::fmt::Display for ExpressionResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(expression-result)")
    }
}

impl<'b, 'h> Root<'b, 'h> for ExpressionResult {
    type Output = CheckedBlock;
    fn advance(
        &mut self,
        _: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'b, 'h>, CheckedBlock>, Error> {
        Ok(Poll::Complete(returned.expect("completed root expression")))
    }
    fn unwind(&mut self, _: &mut Checker<'_, '_>) {}
}
