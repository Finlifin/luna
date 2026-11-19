//! Invocation-demanded selection in an immutable declaration world.
use super::*;
use associated::AssociatedRead;
use comptime_interpretor::{AssociatedSelection, Error};

pub fn resolve_associated_value_for_invocation(
    world: &capabilities::World<'_>,
    access: &n::Expr,
    remaining: &mut u64,
) -> Result<AssociatedSelection, Error> {
    query_budget::invocation(
        remaining,
        Budget {
            steps: usize::MAX,
            ..Budget::default()
        },
        |budget| {
            let (interface, subject, member) = operands(access)?;
            let read = world
                .select_associated_member(interface, member, subject, budget)
                .and_then(|selected| selected.read(budget));
            finish(read, access.source, std::convert::identity)
        },
    )
}

pub(super) fn operands(access: &n::Expr) -> Result<(&n::InterfaceInstance, &Term, Id), Error> {
    let K::AssociatedValue {
        interface,
        subject,
        member,
    } = &access.kind
    else {
        return Err(source_queries::invocation_failure(
            "expected associated value access".into(),
            access.source,
        ));
    };
    Ok((interface, subject, *member))
}

pub(super) fn finish<M>(
    read: Result<AssociatedRead<M>, nir::NominalAdmissionError>,
    source: n::SourceRange,
    demand: impl FnOnce(M) -> nir::associated_initializers::AssociatedSourceDemand,
) -> Result<AssociatedSelection, Error> {
    match read.map_err(|failure| source_queries::invocation_failure(failure, source))? {
        AssociatedRead::Ready(resolution) => Ok(resolution.supply),
        AssociatedRead::NeedsSource(missing) => Err(Error {
            kind: comptime_interpretor::ErrorKind::MissingAssociatedSource(Box::new(demand(
                missing,
            ))),
            source: Some(source),
        }),
    }
}
