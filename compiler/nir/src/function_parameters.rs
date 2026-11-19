//! Declaration domains for callable clause inputs, independent of executable
//! parameters and bodies. Runtime instantiation completes dependent input domains.
use crate::{DeclarationId, DeclarationParameter, Term, TypeError};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub declaration: DeclarationId,
    pub parameters: Vec<DeclarationId>,
    pub domains: Vec<Term>,
}

impl Declaration {
    pub fn binders(&self) -> Result<Vec<DeclarationParameter>, TypeError> {
        crate::declaration_parameters(&self.parameters, &self.domains)
    }
}

/// The lexical frame of a source function's body, contracts and defaults.
pub fn ordinary_frame(
    signature: &Term,
    fuel: &mut usize,
) -> Result<Vec<crate::Sort>, crate::type_computations::Error> {
    use crate::type_computations::Error;
    let Term::Function { parameters, .. } = signature.callable_signature() else {
        return Err(Error::Scope(TypeError::InvalidFunctionSignature));
    };
    if *fuel < parameters.len() {
        *fuel = 0;
        return Err(Error::Budget);
    }
    *fuel -= parameters.len();
    Ok(parameters.iter().map(|parameter| parameter.sort).collect())
}

/// Borrow domains in declaration order. Missing entries use the compact Type domain.
pub fn ordered_domains<'a>(
    domains: &'a std::collections::BTreeMap<DeclarationId, Term>,
    parameters: &[DeclarationId],
    fuel: &mut usize,
) -> Result<Vec<&'a Term>, crate::type_computations::Error> {
    *fuel = fuel
        .checked_sub(parameters.len())
        .ok_or(crate::type_computations::Error::Budget)?;
    Ok(parameters
        .iter()
        .map(|id| {
            domains
                .get(id)
                .unwrap_or(&Term::Primitive(crate::Primitive::Type))
        })
        .collect())
}

/// Admit closed callable declaration inputs using their declared domains.
/// Body preconditions and type requirements are separate invocation checks.
pub fn validate_arguments(
    package: &crate::program::Package,
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), String> {
    crate::validate_declaration_arguments_with_collections_in(
        parameters,
        domains,
        arguments,
        fuel,
        |ty, variant, fuel| crate::normal_forms::query_value_fields(package, ty, variant, fuel),
        |_, _, _| false,
        |domain, fuel| crate::normal_forms::list_element_budgeted(package, domain, fuel),
        package.target,
    )
    .map_err(|error| error.to_string())
}

impl fmt::Display for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(function-parameters (declaration {}) (inputs",
            self.declaration
        )?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (domains")?;
        for domain in &self.domains {
            write!(f, " {domain}")?;
        }
        f.write_str("))")
    }
}

/// Bind the identities of an already-formed declaration group by slot. Method
/// domains and capability arguments may refer to the same inherited prefix.
pub struct DeclarationFrame {
    indices: std::collections::BTreeMap<DeclarationId, (usize, crate::Sort)>,
}

impl DeclarationFrame {
    pub fn new(
        binders: impl ExactSizeIterator<Item = DeclarationParameter>,
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        use crate::type_computations::Error;
        *fuel = fuel.checked_sub(binders.len()).ok_or(Error::Budget)?;
        Ok(Self {
            indices: crate::types::declaration_parameter_indices(binders).map_err(Error::Scope)?,
        })
    }

    /// Terms here may have rigid captures but no surrounding positional frame.
    /// Nested signatures bind their own inputs as usual.
    pub fn abstract_term(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        use crate::type_computations::{Error, charge_term};
        charge_term(term, fuel)?;
        term.check_scope(&[]).map_err(Error::Scope)?;
        term.abstract_parameter_indices(&self.indices)
            .map_err(Error::Scope)
    }
}

impl fmt::Display for DeclarationFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(declaration-frame")?;
        for (id, (slot, sort)) in &self.indices {
            write!(f, " (binding {id} (slot {slot}) (sort {sort}))")?;
        }
        f.write_str(")")
    }
}

/// Check alpha-equivalence of implementation input domains before transporting
/// positional arguments to a method declared in another phase contribution.
/// This validates the input telescope only, not head or contract compatibility.
/// Missing domain metadata denotes the existing compact all-Type convention.
pub fn equivalent_prefix_domains(
    expected: &[DeclarationId],
    actual: &[DeclarationId],
    domains: &std::collections::BTreeMap<DeclarationId, Term>,
    fuel: &mut usize,
) -> Result<bool, crate::type_computations::Error> {
    use crate::type_computations::Error;
    if expected.len() != actual.len() {
        return Ok(false);
    }
    let default = Term::Primitive(crate::Primitive::Type);
    let mut compare = || {
        let mut frame = |ids: &[DeclarationId]| {
            DeclarationFrame::new(
                ids.iter().map(|&id| {
                    DeclarationParameter::comptime(id, domains.get(&id).unwrap_or(&default))
                }),
                fuel,
            )
        };
        let left = frame(expected)?;
        let right = frame(actual)?;
        for (a, b) in expected.iter().zip(actual) {
            let a = domains.get(a).unwrap_or(&default);
            let b = domains.get(b).unwrap_or(&default);
            if a.sort() != crate::Sort::Type
                || b.sort() != crate::Sort::Type
                || left.abstract_term(a, fuel)? != right.abstract_term(b, fuel)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    };
    match compare() {
        Err(Error::Scope(_)) => Ok(false),
        result => result,
    }
}
