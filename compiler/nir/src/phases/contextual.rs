//! Admission of source interfaces for contextual code and the specifications
//! evaluated during construction. Code inputs retain their runtime domains.
use super::{Error, storage_cache::StorageQueries};
use crate::code_interfaces::{CodeDomain, CodeFunctionSignature, CodeInput, SignatureOutput};
use crate::program::MethodContract;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn validate_method_frame<O: SignatureOutput>(
    method: &MethodContract,
    signature: &CodeFunctionSignature<O>,
    fuel: &mut usize,
) -> Result<(), Error> {
    let source = method.source;
    let budget = || Error::budget(source, "contextual method frame budget exhausted");
    signature.charge(fuel).map_err(|failure| match failure {
        crate::type_computations::Error::Budget => budget(),
        _ => Error::invalid(source, "invalid contextual method input telescope"),
    })?;
    if method.type_parameters.len() != signature.clause_domains.len()
        || method.parameter_bindings.len() != signature.inputs.len()
    {
        return Err(Error::invalid(
            source,
            "contextual method frame arity mismatch",
        ));
    }
    if !matches!(signature.output.domain(), CodeDomain::Data(_))
        && method.program.result_binding.is_some()
    {
        return Err(Error::invalid(
            source,
            "quoted result cannot supply a data result binding",
        ));
    }
    let mut inputs = BTreeMap::new();
    for ((binding, input), expected) in signature.inputs.iter().zip(&method.parameter_bindings) {
        if binding != expected || inputs.insert(*binding, input).is_some() {
            return Err(Error::invalid(
                source,
                "contextual method input identity mismatch",
            ));
        }
    }
    *fuel = fuel
        .checked_sub(method.program.entry_snapshots.len())
        .ok_or_else(budget)?;
    if method
        .program
        .entry_snapshots
        .iter()
        .any(|entry| !matches!(inputs.get(&entry.parameter), Some(CodeInput::Data(_))))
    {
        return Err(Error::invalid(
            source,
            "contextual method entry snapshot requires a data parameter",
        ));
    }
    Ok(())
}

pub(super) fn inspect_contract<O: SignatureOutput>(
    storage: &mut StorageQueries<'_>,
    method: &MethodContract,
    signature: &CodeFunctionSignature<O>,
    fuel: &mut usize,
) -> Result<(), Error> {
    let source = method.source;
    let storage_error = |failure| Error::storage(source, failure);
    let mut code_bindings = BTreeSet::new();
    let domains = signature
        .clause_domains
        .iter()
        .map(|ty| (None, CodeDomain::Data(ty)))
        .chain(
            signature
                .inputs
                .iter()
                .map(|(id, input)| (Some(*id), input.domain())),
        )
        .chain(std::iter::once((None, signature.output.domain())));
    for (binding, domain) in domains {
        if let CodeDomain::Data(ty) = domain {
            storage
                .check_comptime(ty, &[], fuel)
                .map_err(storage_error)?;
            continue;
        }
        for ty in domain.type_terms() {
            storage
                .check_runtime(ty, &[], fuel)
                .map_err(storage_error)?;
        }
        if !method.program.preconditions.is_empty() || !method.program.postconditions.is_empty() {
            code_bindings.extend(binding);
            if let CodeDomain::Expression(interface) = domain {
                code_bindings.extend(interface.context.keys().copied());
            }
        }
    }
    if !code_bindings.is_empty() {
        for condition in method
            .program
            .preconditions
            .iter()
            .chain(&method.program.postconditions)
        {
            let free = crate::free_bindings::collect(condition, fuel).map_err(|failure| {
                use crate::free_bindings::Error::*;
                match failure {
                    Budget(message) | Depth(message) => Error::budget(condition.source, message),
                    Invalid(message) => Error::invalid(condition.source, message),
                }
            })?;
            if !free.is_disjoint(&code_bindings) {
                return Err(Error::invalid(
                    condition.source,
                    "contextual contract reads code or its runtime context as ordinary data",
                ));
            }
        }
    }
    Ok(())
}
