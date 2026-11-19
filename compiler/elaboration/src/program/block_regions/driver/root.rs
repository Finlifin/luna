//! Each checking region retains the consumer of its final result.
use super::*;

/// Only the root that requested VM work can consume its result.
pub(in crate::program::block_regions) trait Execution:
    std::fmt::Display
{
    type Reply;
    fn execute(self, steps: &mut u64) -> Result<Self::Reply, Error>;
}

impl Execution for std::convert::Infallible {
    type Reply = Self;
    fn execute(self, _: &mut u64) -> Result<Self, Error> {
        match self {}
    }
}

/// Expression children always return CheckedBlock. The root owns the consumer
/// that can turn those children into a different result in the original scope.
pub(in crate::program::block_regions) trait Root<'h>:
    std::fmt::Display
{
    type Output;
    type Invocation: Execution;
    fn resume(
        &mut self,
        checker: &mut Checker<'_, '_>,
        reply: <Self::Invocation as Execution>::Reply,
    ) -> Result<(), Error>;
    fn advance(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h>, Self::Output, Self::Invocation>, Error>;
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

impl<'h> Root<'h> for ExpressionResult {
    type Output = CheckedBlock;
    type Invocation = std::convert::Infallible;
    fn resume(
        &mut self,
        _: &mut Checker<'_, '_>,
        reply: std::convert::Infallible,
    ) -> Result<(), Error> {
        match reply {}
    }
    fn advance(
        &mut self,
        _: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Poll<Request<'h>, CheckedBlock>, Error> {
        Ok(Poll::Complete(returned.expect("completed root expression")))
    }
    fn unwind(&mut self, _: &mut Checker<'_, '_>) {}
}
