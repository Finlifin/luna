//! Observe public member signatures through ordinary declaration input
//! substitution, without choosing an implementation or normalizing projections.
//! Producers and validators share this bounded contract reconstruction.
use crate::{
    DeclarationId, Primitive, Sort, Term,
    code_interfaces::MethodSignature,
    program::{AssociatedContract, MethodContract, Trait},
    type_computations::{Error, charge_term},
};

fn arguments(
    subject: &Term,
    interface: &[Term],
    method: &[Term],
    fuel: &mut usize,
) -> Result<Vec<Term>, Error> {
    *fuel = fuel
        .checked_sub(
            interface
                .len()
                .saturating_add(method.len())
                .saturating_add(1),
        )
        .ok_or(Error::Budget)?;
    // Preflight the complete input vector before its first owned copy.
    for value in std::iter::once(subject).chain(interface).chain(method) {
        charge_term(value, fuel)?;
    }
    Ok(std::iter::once(subject.clone())
        .chain(interface.iter().cloned())
        .chain(method.iter().cloned())
        .collect())
}

pub fn associated(
    contract: &Trait,
    member: &AssociatedContract,
    subject: &Term,
    inputs: &[Term],
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Term, Error> {
    let arguments = arguments(subject, inputs, &[], fuel)?;
    contract
        .input_substitution(&arguments, frames, fuel)?
        .instantiate_bounded(&member.ty, fuel)
}

pub fn method<'a>(
    contract: &Trait,
    method: &MethodContract,
    subject: &Term,
    interface_inputs: &[Term],
    method_inputs: &[Term],
    mut domain: impl FnMut(DeclarationId, &mut usize) -> Result<Option<&'a Term>, Error>,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<MethodSignature, Error> {
    if interface_inputs.len() != contract.type_parameters.len()
        || method_inputs.len() != method.type_parameters.len()
    {
        return Err(Error::Structure("trait method input arity mismatch".into()));
    }
    *fuel = fuel
        .checked_sub(
            contract
                .type_parameters
                .len()
                .saturating_add(method.type_parameters.len())
                .saturating_add(1),
        )
        .ok_or(Error::Budget)?;
    let parameters: Vec<_> = std::iter::once(contract.self_parameter)
        .chain(contract.type_parameters.iter().copied())
        .chain(method.type_parameters.iter().copied())
        .collect();
    let mut domains = Vec::with_capacity(parameters.len());
    for id in &parameters {
        let domain = domain(*id, fuel)?.unwrap_or(&Term::Primitive(Primitive::Type));
        charge_term(domain, fuel)?;
        domains.push(domain.clone());
    }
    let arguments = arguments(subject, interface_inputs, method_inputs, fuel)?;
    method.signature.instantiate_declaration_inputs_bounded(
        &parameters,
        &domains,
        &arguments,
        frames,
        fuel,
    )
}
