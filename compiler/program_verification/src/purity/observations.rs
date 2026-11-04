//! Deferred P3 obligations keep the declaration's type-premise scope. Finite
//! type coverage specializes only a private analysis view, never source NIR.
use super::*;

#[derive(Clone, Debug, Default)]
pub struct ObservationScope {
    pub parameters: Vec<nir::DeclarationParameter>,
    pub input_scope: Vec<Sort>,
    pub requirements: Vec<n::TypePredicate>,
}

#[derive(Clone, Debug)]
pub struct StableObservation {
    pub value: n::Expr,
    pub source: n::SourceRange,
    pub staged_parameters: BTreeSet<n::LocalId>,
    pub scope: ObservationScope,
}

impl std::fmt::Display for ObservationScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(observation-scope (parameters")?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (input-scope")?;
        for sort in &self.input_scope {
            write!(f, " {sort}")?;
        }
        f.write_str(") (requirements")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for StableObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(stable-observation (source {} {}) {} (staged-parameters",
            self.source.start, self.source.end, self.scope
        )?;
        for binding in &self.staged_parameters {
            write!(f, " {binding}")?;
        }
        write!(f, ") (value {}))", self.value)
    }
}

pub fn validate_defaults<'a>(
    package: &n::Package,
    obligations: impl IntoIterator<Item = &'a StableObservation>,
    protocols: &Protocols,
) -> Result<(), Error> {
    validate_with_budget(package, obligations, protocols, &mut 100_000)
}

fn validate_with_budget<'a>(
    package: &n::Package,
    obligations: impl IntoIterator<Item = &'a StableObservation>,
    protocols: &Protocols,
    fuel: &mut usize,
) -> Result<(), Error> {
    let mut world = None;
    for obligation in obligations {
        if !validate_observation(package, obligation, protocols, fuel, &mut world) {
            return Err(error(
                obligation.source,
                "default expression requires P3 stability/termination evidence; call graph or operations could not be certified",
            ));
        }
    }
    Ok(())
}

fn validate_observation(
    package: &n::Package,
    obligation: &StableObservation,
    protocols: &Protocols,
    fuel: &mut usize,
    world: &mut Option<ResourceWorld>,
) -> bool {
    let scope = &obligation.scope;
    let Some(remaining) = fuel.checked_sub(scope.parameters.len().saturating_add(1)) else {
        *fuel = 0;
        return false;
    };
    *fuel = remaining;
    let inputs: Vec<_> = scope
        .parameters
        .iter()
        .map(|parameter| Term::Rigid {
            declaration: parameter.declaration,
            sort: parameter.sort,
        })
        .collect();
    let cases =
        type_domains::cases_in_scope(&scope.requirements, &scope.input_scope, &inputs, fuel);
    // Unknown coverage retains the original generic obligation. A trait
    // premise alone is not representation, cleanup or termination evidence.
    let Some(cases) = cases else {
        return check_expression(
            package,
            &obligation.value,
            obligation,
            protocols,
            fuel,
            world,
        );
    };
    for inputs in cases {
        if nir::copy_budget::charge_substitution(
            &obligation.value,
            &scope.parameters,
            &inputs,
            fuel,
        )
        .is_err()
        {
            *fuel = 0;
            return false;
        }
        let Ok(value) = obligation.value.instantiate_parameters_in_scope(
            &scope.parameters,
            &inputs,
            &[scope.input_scope.clone()],
        ) else {
            return false;
        };
        if !check_expression(package, &value, obligation, protocols, fuel, world) {
            return false;
        }
    }
    *fuel != 0
}

fn check_expression(
    package: &n::Package,
    value: &n::Expr,
    obligation: &StableObservation,
    protocols: &Protocols,
    fuel: &mut usize,
    world: &mut Option<ResourceWorld>,
) -> bool {
    // Only immutable declaration indexing is shared. Behavior/callable proofs
    // and staged bindings belong to this one case of this one obligation.
    let mut analysis = Analysis::with_fuel(package, true, *fuel);
    analysis.protocols = protocols.clone();
    analysis.resource_world = world.clone();
    let Some(remaining) = analysis
        .fuel
        .checked_sub(obligation.staged_parameters.len())
    else {
        *fuel = 0;
        return false;
    };
    analysis.fuel = remaining;
    analysis
        .staged_parameters
        .extend(&obligation.staged_parameters);
    let valid = analysis.fuel != 0 && analysis.default_expression(value);
    *fuel = analysis.fuel;
    *world = analysis.resource_world;
    valid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obligations_share_fuel_and_recover_in_a_new_validation() {
        let package = n::Package::default();
        let protocols = Protocols::default();
        let source = n::SourceRange { start: 4, end: 9 };
        let obligation = StableObservation {
            value: n::Expr {
                source,
                ty: p(P::Bool),
                kind: K::Literal(n::Literal::Bool(true)),
            },
            source,
            staged_parameters: BTreeSet::new(),
            scope: ObservationScope::default(),
        };
        let mut fuel = 1000;
        validate_with_budget(&package, [&obligation], &protocols, &mut fuel).unwrap();
        let cost = 1000 - fuel;
        assert!(cost > 0);
        assert!(validate_with_budget(&package, [&obligation], &protocols, &mut 0).is_err());
        // New per-obligation analyses cannot replenish the caller's allowance.
        let mut fuel = cost;
        assert!(
            validate_with_budget(&package, [&obligation, &obligation], &protocols, &mut fuel)
                .is_err()
        );
        validate_with_budget(&package, [&obligation], &protocols, &mut 1000).unwrap();
    }
}
