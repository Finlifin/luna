//! Small shape fixtures use an explicit allowance. Resource failure is a test
//! failure, never the expected `false` result for an unsupported domain.
#![allow(dead_code)]
use nir::{DeclarationId, DeclarationParameter, Sort, Term};

pub fn supported_input_type(ty: &Term, prior: &[DeclarationId]) -> bool {
    supported_input_type_with_layouts(ty, prior, |_| false, |_| false)
}
pub fn supported_input_type_with_layouts(
    ty: &Term,
    prior: &[DeclarationId],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
) -> bool {
    let prior: Vec<_> = prior
        .iter()
        .map(|id| DeclarationParameter {
            declaration: *id,
            sort: Sort::Type,
        })
        .collect();
    supported_input_type_in_scope(ty, &prior, is_tuple, is_nominal)
}
pub fn supported_input_type_in_scope(
    ty: &Term,
    prior: &[DeclarationParameter],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
) -> bool {
    supported_input_type_with_collections_in(ty, prior, is_tuple, |_| false, is_nominal, None)
}
pub fn supported_input_type_with_collections_in(
    ty: &Term,
    prior: &[DeclarationParameter],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_list: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
    target: Option<nir::target::TargetData>,
) -> bool {
    nir::normal_forms::supported_input_type_with_collections_budgeted(
        ty,
        prior,
        is_tuple,
        is_list,
        is_nominal,
        target,
        &mut 100_000,
    )
    .expect("shape fixture exhausted its inspection allowance")
}
