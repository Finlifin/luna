//! Metadata checks for opaque definitions. Neither bodies nor heads reduce.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    Budget,
    Invalid(&'static str),
    Newtype(crate::newtypes::ValidationError),
}
impl ValidationError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Budget => "normal-form metadata validation budget exhausted",
            Self::Invalid(message) => message,
            Self::Newtype(error) => error.message(),
        }
    }
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for ValidationError {}

fn spend(fuel: &mut usize, amount: usize) -> Result<(), ValidationError> {
    *fuel = fuel.checked_sub(amount).ok_or(ValidationError::Budget)?;
    Ok(())
}
fn charge_term(term: &Term, fuel: &mut usize) -> Result<(), ValidationError> {
    crate::type_computations::charge_term(term, fuel).map_err(|_| ValidationError::Budget)
}
fn charge_predicate(predicate: &TypePredicate, fuel: &mut usize) -> Result<(), ValidationError> {
    crate::copy_budget::charge_predicate_substitution(predicate, &[], &[], fuel)
        .map_err(|_| ValidationError::Budget)
}

/// Validate the currently supported telescope and its executable-body metadata.
/// External opaque terms need not supply a local body; local head definitions do.
pub fn validate(package: &Package) -> Result<(), &'static str> {
    validate_in(package, &mut 100_000).map_err(|error| error.message())
}

pub fn validate_in(package: &Package, fuel: &mut usize) -> Result<(), ValidationError> {
    use ValidationError::Invalid;
    crate::newtypes::validate_in(package, fuel).map_err(ValidationError::Newtype)?;
    spend(fuel, package.functions.len())?;
    spend(fuel, package.normal_forms.len())?;
    spend(fuel, package.structs.len())?;
    spend(fuel, package.enums.len())?;
    spend(fuel, package.newtypes.len())?;
    let nominal: BTreeSet<_> = package
        .structs
        .iter()
        .map(|s| s.declaration)
        .chain(package.enums.iter().map(|e| e.declaration))
        .chain(package.newtypes.iter().map(|n| n.declaration))
        .collect();
    let mut descriptors = BTreeMap::new();
    for head in &package.normal_forms {
        if descriptors.insert(head.declaration, head).is_some() {
            return Err(Invalid("duplicate normal-form head metadata"));
        }
    }
    let mut functions = BTreeMap::new();
    for function in &package.functions {
        if functions.insert(function.declaration, function).is_some() {
            return Err(Invalid("duplicate function declaration"));
        }
    }
    for head in &package.normal_forms {
        spend(fuel, head.parameters.len())?;
        for parameter in &head.parameters {
            charge_term(&parameter.ty, fuel)?;
        }
        if head.role == Role::Tuple
            && (head.parameters.len() != 1
                || head.parameters[0].kind != Kind::Value
                || !matches!(&head.parameters[0].ty, Term::NormalForm { head: list, .. }
                    if descriptors.get(list).is_some_and(|list| list.is_type_list_instance(&head.parameters[0].ty))))
        {
            return Err(Invalid("Tuple head requires exactly one Type-list input"));
        }
        if head.role == Role::TypeList
            && (head.parameters.len() != 1 || head.parameters[0].kind != Kind::Type)
        {
            return Err(Invalid(
                "meta.List binding has an invalid constructor signature: expected exactly one Type input",
            ));
        }
        if head
            .collector
            .is_some_and(|index| index.checked_add(1) != Some(head.parameters.len()))
        {
            return Err(Invalid("normal-form collector must be the final input"));
        }
        let function = functions.get(&head.declaration);
        if function.is_some_and(|f| f.phase != FunctionPhase::NormalForm || !f.pure) {
            return Err(Invalid(
                "normal-form metadata requires a pure opaque definition",
            ));
        }
        let mut declarations = BTreeSet::new();
        let mut bindings = BTreeSet::new();
        let mut types = Vec::new();
        let mut values = Vec::new();
        let mut prior = Vec::new();
        for parameter in &head.parameters {
            if !declarations.insert(parameter.declaration) || !bindings.insert(parameter.binding) {
                return Err(Invalid("duplicate normal-form input identity"));
            }
            match parameter.kind {
                Kind::Type => {
                    if parameter.ty != Term::Primitive(Primitive::Type) {
                        return Err(Invalid("normal-form Type input must have Type type"));
                    }
                    types.push(parameter.declaration);
                }
                Kind::Value => {
                    let supported = supported_input_type_with_collections_budgeted(
                        &parameter.ty,
                        &prior,
                        |id| {
                            descriptors
                                .get(&id)
                                .is_some_and(|head| head.role == Role::Tuple)
                        },
                        |id| {
                            descriptors
                                .get(&id)
                                .is_some_and(|head| head.role == Role::TypeList)
                        },
                        |id| nominal.contains(&id),
                        package.target,
                        fuel,
                    )
                    .map_err(|_| ValidationError::Budget)?;
                    if !supported {
                        return Err(Invalid(
                            "normal-form value input has an unsupported or forward-dependent type",
                        ));
                    }
                    values.push(parameter);
                }
            }
            prior.push(crate::DeclarationParameter {
                declaration: parameter.declaration,
                sort: if parameter.kind == Kind::Type {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                },
            });
        }
        spend(fuel, head.requirements.len())?;
        for predicate in &head.requirements {
            charge_predicate(predicate, fuel)?;
        }
        // Head declarations can accompany a required-computation package
        // without transporting their non-invocable definition bodies.
        let Some(function) = function else {
            continue;
        };
        // Formation must retain both ordinary predicates and trait capability
        // obligations, including when the non-invocable body is omitted later.
        spend(fuel, function.requirements.len())?;
        for predicate in &function.requirements {
            charge_predicate(predicate, fuel)?;
        }
        spend(fuel, function.capabilities.len())?;
        for capability in &function.capabilities {
            charge_term(&capability.subject, fuel)?;
            spend(fuel, capability.interface_arguments.len())?;
            for argument in &capability.interface_arguments {
                charge_term(argument, fuel)?;
            }
        }
        charge_term(&function.signature, fuel)?;
        spend(fuel, function.type_parameters.len())?;
        spend(fuel, function.parameters.len())?;
        let count = function
            .requirements
            .len()
            .checked_add(function.capabilities.len())
            .ok_or(Invalid("normal-form contract count overflow"))?;
        if head.requirements.len() != count
            || head.requirements[..function.requirements.len()] != function.requirements
            || !head.requirements[function.requirements.len()..].iter().zip(&function.capabilities)
                .all(|(predicate, capability)| matches!(predicate, TypePredicate::Trait(retained) if retained == capability))
        {
            return Err(Invalid("normal-form contracts disagree with the checked definition"));
        }
        if function.type_parameters != types
            || !function
                .parameters
                .iter()
                .copied()
                .eq(values.iter().map(|p| p.binding))
        {
            return Err(Invalid(
                "normal-form input identities disagree with the checked body",
            ));
        }
        let Term::Function { parameters, result } = &function.signature else {
            return Err(Invalid("normal-form body signature is not a function"));
        };
        if **result != Term::Primitive(Primitive::Type)
            || parameters.len() != values.len()
            || parameters
                .iter()
                .zip(values)
                .any(|(actual, input)| actual.sort != Sort::ComptimeValue || actual.ty != input.ty)
        {
            return Err(Invalid(
                "normal-form body signature disagrees with its input telescope",
            ));
        }
    }
    if package
        .functions
        .iter()
        .any(|f| f.phase == FunctionPhase::NormalForm && !descriptors.contains_key(&f.declaration))
    {
        return Err(Invalid(
            "normal-form definition is missing its ordered input metadata",
        ));
    }
    Ok(())
}
