//! Stable ordered input metadata for opaque definitions. No body reduction.
use crate::program::{FunctionPhase, LocalId, Package, TypePredicate};
use crate::{DeclarationId, Primitive, Sort, Term};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Compiler-provided callable identities and their ordinary signatures.
/// This registry describes builtin exports; it does not change NF application,
/// argument representation, or matching for any declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinCallable {
    List,
    Tuple,
}
impl BuiltinCallable {
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "List" => Some(Self::List),
            "Tuple" => Some(Self::Tuple),
            _ => None,
        }
    }

    pub fn signature(self, list: Option<DeclarationId>) -> Option<Term> {
        let (sort, domain, collector) = match self {
            Self::List => (Sort::Type, Term::Primitive(Primitive::Type), None),
            Self::Tuple => (
                Sort::ComptimeValue,
                Term::NormalForm {
                    head: list?,
                    sort: Sort::Type,
                    arguments: vec![Term::Primitive(Primitive::Type)],
                },
                Some(0),
            ),
        };
        Some(Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![crate::Parameter { sort, ty: domain }],
                result: Box::new(Term::Primitive(Primitive::Type)),
            }),
            collector,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Type,
    Value,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub declaration: DeclarationId,
    pub binding: LocalId,
    pub kind: Kind,
    pub ty: Term,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    Opaque,
    Tuple,
    /// Compiler-bound immutable meta.List constructor; the head stays opaque.
    TypeList,
}
impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Opaque => "opaque",
            Self::Tuple => "tuple",
            Self::TypeList => "type-list",
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// Position collected at the source call boundary, independent of domain.
    pub collector: Option<usize>,
    /// Trusted language-item contract, independent of the definition body.
    pub role: Role,
    pub declaration: DeclarationId,
    pub parameters: Vec<Parameter>,
    pub requirements: Vec<TypePredicate>,
}
impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Type => "type",
            Self::Value => "value",
        })
    }
}
impl fmt::Display for Parameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-form-input (declaration {}) (binding {}) (kind {}) (type {}))",
            self.declaration, self.binding, self.kind, self.ty
        )
    }
}
impl fmt::Display for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(normal-form-head (declaration {}) (role {}) (inputs",
            self.declaration, self.role
        )?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (collector ")?;
        match self.collector {
            Some(index) => write!(f, "{index}")?,
            None => f.write_str("none")?,
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str("))")
    }
}

/// Builtin immutable inputs, including named/positional products and prior Type dependencies.
/// This is a representation check, not a proof about opaque user types.
pub fn supported_input_type(ty: &Term, prior_types: &[DeclarationId]) -> bool {
    supported_input_type_with_tuple_heads(ty, prior_types, |_| false)
}

/// The caller supplies trusted Tuple identities, never names or body shapes.
pub fn supported_input_type_with_tuple_heads(
    ty: &Term,
    prior_types: &[DeclarationId],
    is_tuple: impl Fn(DeclarationId) -> bool,
) -> bool {
    supported_input_type_with_layouts(ty, prior_types, is_tuple, |_| false)
}
pub fn supported_input_type_with_layouts(
    ty: &Term,
    prior_types: &[DeclarationId],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
) -> bool {
    let prior = prior_types
        .iter()
        .map(|declaration| crate::DeclarationParameter {
            declaration: *declaration,
            sort: Sort::Type,
        })
        .collect::<Vec<_>>();
    supported_input_type_in_scope(ty, &prior, is_tuple, is_nominal)
}

/// Ordered declaration inputs retain their sorts, including value indices in
/// nominal domains. This admits a representation, not a formation obligation.
pub fn supported_input_type_in_scope(
    ty: &Term,
    prior: &[crate::DeclarationParameter],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
) -> bool {
    supported_input_type_in_scope_in(ty, prior, is_tuple, is_nominal, None)
}

pub fn supported_input_type_in_scope_in(
    ty: &Term,
    prior: &[crate::DeclarationParameter],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
    target: Option<crate::target::TargetData>,
) -> bool {
    supported_input_type_with_collections_in(ty, prior, is_tuple, |_| false, is_nominal, target)
}

pub fn supported_input_type_with_collections_in(
    ty: &Term,
    prior: &[crate::DeclarationParameter],
    is_tuple: impl Fn(DeclarationId) -> bool,
    is_list: impl Fn(DeclarationId) -> bool,
    is_nominal: impl Fn(DeclarationId) -> bool,
    target: Option<crate::target::TargetData>,
) -> bool {
    let mut work = vec![(ty, 0usize)];
    let mut remaining = 100_000usize;
    while let Some((ty, depth)) = work.pop() {
        if depth > 128 || remaining == 0 {
            return false;
        }
        remaining -= 1;
        match ty {
            Term::ClosureContextType => {}
            Term::Optional(payload) => {
                if payload.as_ref() != &Term::Primitive(Primitive::Never) {
                    work.push((payload, depth + 1));
                }
            }
            Term::ErrorResult { errors, result } => {
                if errors.is_empty() || errors.len().saturating_add(1) > remaining {
                    return false;
                }
                work.extend(errors.iter().map(|ty| (ty, depth + 1)));
                if result.as_ref() != &Term::Primitive(Primitive::Never) {
                    work.push((result, depth + 1));
                }
            }
            Term::Primitive(
                Primitive::Type
                | Primitive::Void
                | Primitive::Bool
                | Primitive::Integer
                | Primitive::Float
                | Primitive::Char
                | Primitive::Str
                | Primitive::Symbol
                | Primitive::Object
                | Primitive::F16
                | Primitive::Bf16
                | Primitive::F32
                | Primitive::F64
                | Primitive::BitVec(_),
            ) => {}
            Term::Primitive(p)
                if target
                    .map_or_else(
                        || p.fixed_integer_layout(),
                        |target| target.integer_layout(*p),
                    )
                    .is_some() => {}
            Term::Rigid {
                declaration,
                sort: Sort::Type,
            } if prior
                .iter()
                .any(|p| p.declaration == *declaration && p.sort == Sort::Type) => {}
            Term::Nominal { declaration, .. } if is_nominal(*declaration) => {
                if !nominal_domain_scope(ty, prior, &mut remaining) {
                    return false;
                }
            }
            Term::AssociatedType { .. } => {
                // Declaration-side projections are symbolic domains, just as
                // a prior Type input is. Selection must resolve the actual
                // value representation before admitting a completed input.
                if !nominal_domain_scope(ty, prior, &mut remaining) {
                    return false;
                }
            }
            Term::Record(fields) => {
                if fields.len() > remaining {
                    return false;
                }
                let mut names = BTreeSet::new();
                for (name, ty) in fields {
                    let Some(rest) = remaining.checked_sub(name.len()) else {
                        return false;
                    };
                    remaining = rest;
                    if !names.insert(name) {
                        return false;
                    }
                    work.push((ty, depth + 1));
                }
            }
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments,
            } if is_list(*head) => {
                let [element] = arguments.as_slice() else {
                    return false;
                };
                if element.sort() != Sort::Type {
                    return false;
                }
                work.push((element, depth + 1));
            }
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments,
            } if is_tuple(*head) => {
                let [Term::TypeListValue(fields)] = arguments.as_slice() else {
                    return false;
                };
                if fields.len() > remaining {
                    return false;
                }
                work.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            Term::Tuple(fields) => {
                if fields.len() > remaining {
                    return false;
                }
                work.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            _ => return false,
        }
    }
    true
}

/// Representation-shape guard for the list layout. A matching shape alone
/// does not establish a list declaration; consult its registered layout role.
pub fn type_list_input_type(ty: &Term) -> bool {
    let arguments = match ty {
        Term::NormalForm {
            sort: Sort::Type,
            arguments,
            ..
        }
        | Term::Nominal { arguments, .. } => arguments,
        _ => return false,
    };
    arguments.as_slice() == [Term::Primitive(Primitive::Type)]
}

/// Validate the currently supported telescope and its executable-body metadata.
/// External opaque terms need not supply a local body; local head definitions do.
pub fn validate(package: &Package) -> Result<(), &'static str> {
    crate::newtypes::validate(package)?;
    let mut functions = BTreeMap::new();
    for function in &package.functions {
        if functions.insert(function.declaration, function).is_some() {
            return Err("duplicate function declaration");
        }
    }
    let mut heads = BTreeSet::new();
    for head in &package.normal_forms {
        if !heads.insert(head.declaration) {
            return Err("duplicate normal-form head metadata");
        }
        if head.role == Role::Tuple
            && (head.parameters.len() != 1
                || head.parameters[0].kind != Kind::Value
                || !package
                    .normal_forms
                    .iter()
                    .any(|list| list.is_type_list_instance(&head.parameters[0].ty)))
        {
            return Err("Tuple head requires exactly one Type-list input");
        }
        if head.role == Role::TypeList
            && (head.parameters.len() != 1 || head.parameters[0].kind != Kind::Type)
        {
            return Err(
                "meta.List binding has an invalid constructor signature: expected exactly one Type input",
            );
        }
        if head
            .collector
            .is_some_and(|index| index.checked_add(1) != Some(head.parameters.len()))
        {
            return Err("normal-form collector must be the final input");
        }
        let function = functions.get(&head.declaration);
        if function.is_some_and(|f| f.phase != FunctionPhase::NormalForm || !f.pure) {
            return Err("normal-form metadata requires a pure opaque definition");
        }
        let mut declarations = BTreeSet::new();
        let mut bindings = BTreeSet::new();
        let mut types = Vec::new();
        let mut values = Vec::new();
        let mut prior = Vec::new();
        for parameter in &head.parameters {
            if !declarations.insert(parameter.declaration) || !bindings.insert(parameter.binding) {
                return Err("duplicate normal-form input identity");
            }
            match parameter.kind {
                Kind::Type => {
                    if parameter.ty != Term::Primitive(Primitive::Type) {
                        return Err("normal-form Type input must have Type type");
                    }
                    types.push(parameter.declaration);
                }
                Kind::Value => {
                    let supported = supported_input_type_with_collections_in(
                        &parameter.ty,
                        &prior,
                        |id| {
                            package
                                .normal_forms
                                .iter()
                                .any(|head| head.declaration == id && head.role == Role::Tuple)
                        },
                        |id| {
                            package
                                .normal_forms
                                .iter()
                                .any(|head| head.declaration == id && head.role == Role::TypeList)
                        },
                        |id| {
                            package.structs.iter().any(|s| s.declaration == id)
                                || package.enums.iter().any(|e| e.declaration == id)
                                || package.newtypes.iter().any(|n| n.declaration == id)
                        },
                        package.target,
                    );
                    if !supported {
                        return Err(
                            "normal-form value input has an unsupported or forward-dependent type",
                        );
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
        // Head declarations can accompany a required-computation package
        // without transporting their non-invocable definition bodies.
        let Some(function) = function else {
            continue;
        };
        // Formation must retain both ordinary predicates and trait capability
        // obligations, including when the non-invocable body is omitted later.
        let count = function
            .requirements
            .len()
            .checked_add(function.capabilities.len())
            .ok_or("normal-form contract count overflow")?;
        if head.requirements.len() != count
            || head.requirements[..function.requirements.len()] != function.requirements
            || !head.requirements[function.requirements.len()..].iter().zip(&function.capabilities)
                .all(|(predicate, capability)| matches!(predicate, TypePredicate::Trait(retained) if retained == capability))
        {
            return Err("normal-form contracts disagree with the checked definition");
        }
        if function.type_parameters != types
            || function.parameters != values.iter().map(|p| p.binding).collect::<Vec<_>>()
        {
            return Err("normal-form input identities disagree with the checked body");
        }
        let Term::Function { parameters, result } = &function.signature else {
            return Err("normal-form body signature is not a function");
        };
        if **result != Term::Primitive(Primitive::Type)
            || parameters.len() != values.len()
            || parameters
                .iter()
                .zip(values)
                .any(|(actual, input)| actual.sort != Sort::ComptimeValue || actual.ty != input.ty)
        {
            return Err("normal-form body signature disagrees with its input telescope");
        }
    }
    if package
        .functions
        .iter()
        .any(|f| f.phase == FunctionPhase::NormalForm && !heads.contains(&f.declaration))
    {
        return Err("normal-form definition is missing its ordered input metadata");
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    Arity { expected: usize, actual: usize },
    InputType { index: usize },
    InputDependency { index: usize },
}
impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arity { expected, actual } => write!(
                f,
                "normal-form head expects {expected} inputs, found {actual}"
            ),
            Self::InputType { index } => write!(
                f,
                "normal-form input {index} has an incompatible value or range"
            ),
            Self::InputDependency { index } => write!(
                f,
                "normal-form input {index} depends on an unavailable prior Type input"
            ),
        }
    }
}
impl Head {
    /// Reconstruct the callable's dependent input frame without unfolding the head.
    pub fn callable_type(&self) -> Result<Term, crate::TypeError> {
        let mut binders = Vec::new();
        let mut parameters = Vec::new();
        for input in &self.parameters {
            let sort = if input.kind == Kind::Type {
                Sort::Type
            } else {
                Sort::ComptimeValue
            };
            parameters.push(crate::Parameter {
                sort,
                ty: input.ty.abstract_parameters(&binders)?,
            });
            binders.push(crate::DeclarationParameter {
                declaration: input.declaration,
                sort,
            });
        }
        let ty = Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters,
                result: Box::new(Term::Primitive(Primitive::Type)),
            }),
            self.collector,
        );
        ty.check_scope(&[])?;
        Ok(ty)
    }
    /// Inspect the trusted list representation without unfolding its opaque head.
    pub fn list_element<'a>(&self, ty: &'a Term) -> Option<&'a Term> {
        if self.role != Role::TypeList
            || !matches!(self.parameters.as_slice(), [parameter]
                if parameter.kind == Kind::Type && parameter.ty == Term::Primitive(Primitive::Type))
        {
            return None;
        }
        match ty {
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments,
            } if *head == self.declaration => match arguments.as_slice() {
                [element] if element.sort() == Sort::Type => Some(element),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn is_type_list_instance(&self, ty: &Term) -> bool {
        self.role == Role::TypeList
            && matches!(self.parameters.as_slice(), [parameter]
                if parameter.kind == Kind::Type && parameter.ty == Term::Primitive(Primitive::Type))
            && matches!(ty, Term::NormalForm { head, sort: Sort::Type, arguments }
                if *head == self.declaration && arguments.as_slice() == [Term::Primitive(Primitive::Type)])
    }

    pub fn tuple_elements<'a>(&self, ty: &'a Term) -> Option<&'a [Term]> {
        let Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } = ty
        else {
            return None;
        };
        if self.role != Role::Tuple || *head != self.declaration {
            return None;
        }
        let [Term::TypeListValue(elements)] = arguments.as_slice() else {
            return None;
        };
        Some(elements)
    }
    /// Check concrete inputs without reducing a head. The caller separately
    /// validates term shape, scope and formation contracts.
    pub fn check_closed_arguments(&self, arguments: &[Term]) -> Result<(), ApplicationError> {
        self.check_closed_arguments_with_tuple_heads(arguments, |_| false)
    }

    pub fn check_closed_arguments_with_tuple_heads(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
    ) -> Result<(), ApplicationError> {
        self.check_closed_arguments_with_layouts(arguments, is_tuple, |_, _| None)
    }

    /// Layout lookup must use validated declarations from the caller's world.
    /// Absence of metadata rejects nominal values; type spelling grants no layout.
    pub fn check_closed_arguments_with_layouts(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>) -> Option<Vec<Term>>,
    ) -> Result<(), ApplicationError> {
        self.check_closed_arguments_budgeted(
            arguments,
            is_tuple,
            |ty, variant, _| nominal_fields(ty, variant),
            &mut 100_000,
        )
    }

    pub fn check_closed_arguments_budgeted(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        fuel: &mut usize,
    ) -> Result<(), ApplicationError> {
        self.check_closed_arguments_budgeted_in(arguments, is_tuple, nominal_fields, fuel, None)
    }

    pub fn check_closed_arguments_budgeted_in(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        fuel: &mut usize,
        target: Option<crate::target::TargetData>,
    ) -> Result<(), ApplicationError> {
        self.check_arguments_with_evidence_budgeted_in(
            arguments,
            is_tuple,
            nominal_fields,
            |_, _| false,
            fuel,
            target,
        )
    }

    /// Residual inputs need independent declaration-domain evidence. Closed
    /// argument admission above deliberately supplies no such authority.
    pub fn check_arguments_with_evidence_budgeted(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        declared: impl Fn(&Term, &Term) -> bool,
        fuel: &mut usize,
    ) -> Result<(), ApplicationError> {
        self.check_arguments_with_evidence_budgeted_in(
            arguments,
            is_tuple,
            nominal_fields,
            declared,
            fuel,
            None,
        )
    }

    pub fn check_arguments_with_evidence_budgeted_in(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        declared: impl Fn(&Term, &Term) -> bool,
        fuel: &mut usize,
        target: Option<crate::target::TargetData>,
    ) -> Result<(), ApplicationError> {
        self.check_arguments_in_scope_with_evidence_budgeted_in(
            arguments,
            is_tuple,
            nominal_fields,
            declared,
            &[],
            fuel,
            target,
        )
    }

    /// Callable-local value parameters carry both a checked binder frame and
    /// domain evidence. Substituting a prior Type argument must preserve that
    /// frame rather than requiring the argument to be closed at the root.
    pub fn check_arguments_in_scope_with_evidence_budgeted(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        declared: impl Fn(&Term, &Term) -> bool,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), ApplicationError> {
        self.check_arguments_in_scope_with_evidence_budgeted_in(
            arguments,
            is_tuple,
            nominal_fields,
            declared,
            frames,
            fuel,
            None,
        )
    }

    pub fn check_arguments_in_scope_with_evidence_budgeted_in(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        declared: impl Fn(&Term, &Term) -> bool,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
        target: Option<crate::target::TargetData>,
    ) -> Result<(), ApplicationError> {
        self.check_arguments_in_scope_with_collections_budgeted_in(
            arguments,
            is_tuple,
            |_| false,
            nominal_fields,
            declared,
            frames,
            fuel,
            target,
        )
    }

    /// Collection authority is supplied by the validated declaration world,
    /// never inferred from a frozen value's head spelling or field count.
    pub fn check_arguments_in_scope_with_collections_budgeted_in(
        &self,
        arguments: &[Term],
        is_tuple: impl Fn(DeclarationId) -> bool,
        is_list: impl Fn(DeclarationId) -> bool,
        nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
        declared: impl Fn(&Term, &Term) -> bool,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
        target: Option<crate::target::TargetData>,
    ) -> Result<(), ApplicationError> {
        let mut remaining = *fuel;
        let result = (|| {
            if arguments.len() != self.parameters.len() {
                return Err(ApplicationError::Arity {
                    expected: self.parameters.len(),
                    actual: arguments.len(),
                });
            }
            let mut types: BTreeMap<DeclarationId, (Sort, &Term)> = BTreeMap::new();
            for (index, (parameter, argument)) in self.parameters.iter().zip(arguments).enumerate()
            {
                crate::type_computations::charge_term(argument, &mut remaining)
                    .map_err(|_| ApplicationError::InputType { index })?;
                let before_template = remaining;
                crate::type_computations::charge_term(&parameter.ty, &mut remaining)
                    .map_err(|_| ApplicationError::InputType { index })?;
                let template_work = before_template - remaining;
                let dependencies = parameter
                    .ty
                    .rigid_parameters()
                    .map_err(|_| ApplicationError::InputDependency { index })?;
                let prior = dependencies
                    .keys()
                    .filter_map(|id| {
                        types.get(id).map(|(sort, value)| {
                            (
                                crate::DeclarationParameter {
                                    declaration: *id,
                                    sort: *sort,
                                },
                                *value,
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                let before_inputs = remaining;
                for (_, ty) in &prior {
                    crate::type_computations::charge_term(ty, &mut remaining)
                        .map_err(|_| ApplicationError::InputType { index })?;
                }
                // Bound substitution expansion before cloning prior inputs at
                // each occurrence in the parameter's domain.
                let input_work = before_inputs - remaining;
                remaining = remaining
                    .checked_sub(template_work.saturating_mul(input_work))
                    .ok_or(ApplicationError::InputType { index })?;
                let parameters: Vec<_> = prior
                    .iter()
                    .map(|(parameter, _)| parameter.clone())
                    .collect();
                let inputs: Vec<_> = prior.iter().map(|(_, value)| (*value).clone()).collect();
                let expected = parameter
                    .ty
                    .instantiate_parameters_in_scope(&parameters, &inputs, frames)
                    .map_err(|_| ApplicationError::InputDependency { index })?;
                let mut work = vec![(expected, argument, 0usize)];
                while let Some((expected, argument, depth)) = work.pop() {
                    if depth > 128 || remaining == 0 {
                        return Err(ApplicationError::InputType { index });
                    }
                    remaining -= 1;
                    let accepted =
                        match (&expected, argument) {
                            (
                                _,
                                Term::Rigid {
                                    sort: Sort::ComptimeValue,
                                    ..
                                }
                                | Term::Bound {
                                    sort: Sort::ComptimeValue,
                                    ..
                                },
                            ) => {
                                expected != Term::Primitive(Primitive::Type)
                                    && declared(argument, &expected)
                            }
                            (Term::Optional(payload), Term::OptionalValue { ty, value })
                                if ty.as_ref() == &expected =>
                            {
                                if let Some(value) = value {
                                    work.push(((**payload).clone(), value.as_ref(), depth + 1));
                                }
                                true
                            }
                            (Term::ErrorResult { .. }, Term::ErrorValue { value, .. }) => {
                                let payload = crate::error_results::frozen_payload_domain(
                                    argument,
                                    &expected,
                                    &mut remaining,
                                )
                                .map_err(|_| ApplicationError::InputType { index })?;
                                work.push((payload.clone(), value.as_ref(), depth + 1));
                                true
                            }
                            (
                                Term::NormalForm {
                                    head,
                                    sort: Sort::Type,
                                    arguments,
                                },
                                Term::TypeListValue(fields),
                            ) if is_list(*head)
                                && arguments.as_slice() == [Term::Primitive(Primitive::Type)] =>
                            {
                                if fields.len() > remaining {
                                    return Err(ApplicationError::InputType { index });
                                }
                                work.extend(fields.iter().map(|value| {
                                    (Term::Primitive(Primitive::Type), value, depth + 1)
                                }));
                                true
                            }
                            (
                                Term::NormalForm {
                                    head,
                                    sort: Sort::Type,
                                    arguments,
                                },
                                Term::NominalValue {
                                    ty,
                                    variant: None,
                                    fields,
                                },
                            ) if ty.as_ref() == &expected && is_list(*head) => {
                                let [element] = arguments.as_slice() else {
                                    return Err(ApplicationError::InputType { index });
                                };
                                if element.sort() != Sort::Type
                                    || *element == Term::Primitive(Primitive::Type)
                                    || fields.len() > remaining
                                {
                                    return Err(ApplicationError::InputType { index });
                                }
                                for value in fields {
                                    crate::type_computations::charge_term(element, &mut remaining)
                                        .map_err(|_| ApplicationError::InputType { index })?;
                                    work.push((element.clone(), value, depth + 1));
                                }
                                true
                            }
                            (
                                Term::Nominal { .. },
                                Term::NominalValue {
                                    ty,
                                    variant,
                                    fields,
                                },
                            ) if ty.as_ref() == &expected => {
                                let types = nominal_fields(&expected, *variant, &mut remaining)
                                    .ok_or(ApplicationError::InputType { index })?;
                                if types.len() != fields.len() || fields.len() > remaining {
                                    return Err(ApplicationError::InputType { index });
                                }
                                for (ty, value) in types.into_iter().zip(fields) {
                                    crate::type_computations::charge_term(&ty, &mut remaining)
                                        .map_err(|_| ApplicationError::InputType { index })?;
                                    work.push((ty, value, depth + 1));
                                }
                                true
                            }
                            (
                                Term::Primitive(Primitive::Object),
                                Term::NominalValue {
                                    ty,
                                    variant: None,
                                    fields,
                                },
                            ) if ty.as_ref() == &expected => {
                                if fields.len() > remaining {
                                    return Err(ApplicationError::InputType { index });
                                }
                                for entry in fields {
                                    let (_, domain, value) = crate::objects::entry_parts(entry)
                                        .ok_or(ApplicationError::InputType { index })?;
                                    crate::type_computations::charge_term(domain, &mut remaining)
                                        .map_err(|_| ApplicationError::InputType { index })?;
                                    work.push((domain.clone(), value, depth + 1));
                                }
                                true
                            }
                            (Term::Record(fields), Term::RecordValue(values))
                                if fields.len() == values.len() && fields.len() <= remaining =>
                            {
                                let mut names = BTreeSet::new();
                                for ((name, ty), (label, value)) in fields.iter().zip(values) {
                                    remaining = remaining
                                        .checked_sub(name.len())
                                        .and_then(|n| n.checked_sub(label.len()))
                                        .ok_or(ApplicationError::InputType { index })?;
                                    if name != label || !names.insert(name) {
                                        return Err(ApplicationError::InputType { index });
                                    }
                                    work.push((ty.clone(), value, depth + 1));
                                }
                                true
                            }
                            (
                                Term::NormalForm {
                                    head,
                                    sort: Sort::Type,
                                    arguments,
                                },
                                Term::TupleValue(values),
                            ) if is_tuple(*head) => {
                                let [Term::TypeListValue(fields)] = arguments.as_slice() else {
                                    return Err(ApplicationError::InputType { index });
                                };
                                if fields.len() != values.len() || fields.len() > remaining {
                                    return Err(ApplicationError::InputType { index });
                                }
                                work.extend(
                                    fields
                                        .iter()
                                        .zip(values)
                                        .map(|(ty, value)| (ty.clone(), value, depth + 1)),
                                );
                                true
                            }
                            (Term::Tuple(fields), Term::TupleValue(values))
                                if fields.len() == values.len() && fields.len() <= remaining =>
                            {
                                work.extend(
                                    fields
                                        .iter()
                                        .zip(values)
                                        .map(|(ty, value)| (ty.clone(), value, depth + 1)),
                                );
                                true
                            }
                            (Term::Primitive(Primitive::Type), value) => value.sort() == Sort::Type,
                            (Term::ClosureContextType, Term::ClosureContextValue(_)) => true,
                            (Term::Primitive(Primitive::BitVec(width)), Term::BitVec(value)) => {
                                *width == value.width()
                            }
                            (Term::Primitive(Primitive::Void), Term::Unit) => true,
                            (Term::Primitive(Primitive::Bool), Term::Bool(_)) => true,
                            (Term::Primitive(Primitive::F16), Term::Float16(_)) => true,
                            (Term::Primitive(Primitive::Bf16), Term::BFloat16(_)) => true,
                            (Term::Primitive(Primitive::F32), Term::Float32(_)) => true,
                            (Term::Primitive(Primitive::F64), Term::Float64(_)) => true,
                            (Term::Primitive(Primitive::Integer), Term::Integer(_)) => true,
                            (Term::Primitive(Primitive::Float), Term::Float(_)) => true,
                            (Term::Primitive(Primitive::Char), Term::Char(_)) => true,
                            (Term::Primitive(Primitive::Str), Term::String(_)) => true,
                            (Term::Primitive(Primitive::Symbol), Term::Symbol(_)) => true,
                            (Term::Primitive(primitive), Term::Integer(value)) => target
                                .map_or_else(
                                    || primitive.fixed_integer_layout(),
                                    |target| target.integer_layout(*primitive),
                                )
                                .is_some_and(|(signed, width)| {
                                    if signed {
                                        value.to_signed(width).is_ok()
                                    } else {
                                        value.to_unsigned(width).is_ok()
                                    }
                                }),
                            _ => false,
                        };
                    if !accepted {
                        return Err(ApplicationError::InputType { index });
                    }
                }
                types.insert(
                    parameter.declaration,
                    (
                        if parameter.kind == Kind::Type {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        },
                        argument,
                    ),
                );
            }
            Ok(())
        })();
        *fuel = remaining;
        result
    }
}

/// Borrow tuple fields from a trusted representation contract. Legacy structural
/// tuples remain admitted during migration; other opaque heads have no view.
pub fn tuple_fields<'a>(package: Option<&Package>, ty: &'a Term) -> Option<&'a [Term]> {
    match ty {
        Term::Tuple(fields) => Some(fields),
        Term::NormalForm { head, .. } => package?
            .normal_forms
            .iter()
            .find(|metadata| metadata.declaration == *head)?
            .tuple_elements(ty),
        _ => None,
    }
}

/// Obtain declaration-order fields for a closed nominal value. Callers still
/// check its payloads and formation requirements in their own semantic world.
pub fn nominal_value_fields(
    package: &Package,
    ty: &Term,
    variant: Option<u32>,
) -> Option<Vec<Term>> {
    nominal_value_fields_budgeted(package, ty, variant, &mut 100_000)
}
pub enum NominalValueDeclaration<'a> {
    Struct(&'a crate::program::Struct),
    Enum(&'a crate::program::Enum),
    Newtype(&'a crate::newtypes::Newtype),
}

pub fn nominal_value_fields_budgeted(
    package: &Package,
    ty: &Term,
    variant: Option<u32>,
    fuel: &mut usize,
) -> Option<Vec<Term>> {
    nominal_value_fields_with_lookup_in(
        ty,
        variant,
        fuel,
        &|id, fuel| {
            for definition in &package.enums {
                spend_layout(fuel, 1)?;
                if definition.declaration == id {
                    return Some(NominalValueDeclaration::Enum(definition));
                }
            }
            for definition in &package.newtypes {
                spend_layout(fuel, 1)?;
                if definition.declaration == id {
                    return Some(NominalValueDeclaration::Newtype(definition));
                }
            }
            for definition in &package.structs {
                spend_layout(fuel, 1)?;
                if definition.declaration == id {
                    return Some(NominalValueDeclaration::Struct(definition));
                }
            }
            None
        },
        package.target,
    )
}

/// Borrow checked declarations without cloning a package just to inspect a value.
/// Lookup must charge its work to the shared budget.
pub fn nominal_value_fields_with_lookup<'a>(
    ty: &Term,
    variant: Option<u32>,
    fuel: &mut usize,
    lookup: &impl Fn(DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'a>>,
) -> Option<Vec<Term>> {
    nominal_value_fields_with_lookup_in(ty, variant, fuel, lookup, None)
}
pub fn nominal_value_fields_with_lookup_in<'a>(
    ty: &Term,
    variant: Option<u32>,
    fuel: &mut usize,
    lookup: &impl Fn(DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'a>>,
    target: Option<crate::target::TargetData>,
) -> Option<Vec<Term>> {
    nominal_value_fields_at(ty, variant, fuel, lookup, 0, target)
}
fn nominal_value_fields_at<'a>(
    ty: &Term,
    variant: Option<u32>,
    fuel: &mut usize,
    lookup: &impl Fn(DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'a>>,
    depth: usize,
    target: Option<crate::target::TargetData>,
) -> Option<Vec<Term>> {
    if depth >= 128 {
        return None;
    }
    let layout = |ty: &Term, variant, fuel: &mut usize| {
        nominal_value_fields_at(ty, variant, fuel, lookup, depth + 1, target).ok_or_else(|| {
            if *fuel == 0 {
                crate::NominalAdmissionError::Budget
            } else {
                crate::NominalAdmissionError::Invalid("nominal payload layout unavailable".into())
            }
        })
    };
    let Term::Nominal {
        declaration,
        arguments,
    } = ty
    else {
        return None;
    };
    match (lookup(*declaration, fuel)?, variant) {
        (NominalValueDeclaration::Enum(definition), Some(index))
            if definition.declaration == *declaration =>
        {
            let admitted = crate::validate_declaration_arguments_with_layouts_in(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                fuel,
                layout,
                target,
            );
            layout_admission(admitted, fuel)?;
            instantiate_value_fields(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                definition.variants.get(index as usize)?.fields.iter(),
                fuel,
            )
        }
        (NominalValueDeclaration::Newtype(definition), None)
            if definition.declaration == *declaration =>
        {
            let admitted = definition
                .validate_completed_arguments_with_layouts_in(arguments, fuel, layout, target);
            layout_admission(admitted, fuel)?;
            Some(vec![
                definition
                    .representation_budgeted(arguments, &[], fuel)
                    .ok()?,
            ])
        }
        (NominalValueDeclaration::Struct(definition), None)
            if definition.declaration == *declaration =>
        {
            let admitted = crate::validate_declaration_arguments_with_layouts_in(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                fuel,
                layout,
                target,
            );
            layout_admission(admitted, fuel)?;
            instantiate_value_fields(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                definition.fields.iter().map(|(_, ty)| ty),
                fuel,
            )
        }
        _ => None,
    }
}
// Legacy layout callers use None plus exhausted fuel to distinguish resource
// failure from invalid metadata. Admission can fail a multi-step charge while
// leaving positive fuel, so preserve its typed error before erasing it here.
fn layout_admission<T>(
    result: Result<T, crate::NominalAdmissionError>,
    fuel: &mut usize,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(crate::NominalAdmissionError::Budget) => {
            *fuel = 0;
            None
        }
        Err(crate::NominalAdmissionError::Invalid(_)) => None,
    }
}
fn spend_layout(fuel: &mut usize, work: usize) -> Option<()> {
    match fuel.checked_sub(work) {
        Some(left) => {
            *fuel = left;
            Some(())
        }
        None => {
            *fuel = 0;
            None
        }
    }
}
/// Instantiate only the selected layout, bounding substitution before copying.
/// An empty payload still validates the complete declaration argument frame.
pub fn instantiate_value_fields<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fields: impl ExactSizeIterator<Item = &'a Term>,
    fuel: &mut usize,
) -> Option<Vec<Term>> {
    spend_layout(
        fuel,
        fields
            .len()
            .saturating_add(parameters.len())
            .saturating_add(arguments.len()),
    )?;
    for domain in domains {
        charge_layout_term(domain, fuel)?;
    }
    let parameters = crate::declaration_parameters(parameters, domains).ok()?;
    let before = *fuel;
    for argument in arguments {
        charge_layout_term(argument, fuel)?;
    }
    let input_work = before - *fuel;
    Term::Tuple(vec![])
        .instantiate_parameters(&parameters, arguments)
        .ok()?;
    let mut output = Vec::with_capacity(fields.len());
    for field in fields {
        let before = *fuel;
        charge_layout_term(field, fuel)?;
        spend_layout(fuel, (before - *fuel).saturating_mul(input_work))?;
        output.push(field.instantiate_parameters(&parameters, arguments).ok()?);
    }
    Some(output)
}

fn charge_layout_term(term: &Term, fuel: &mut usize) -> Option<()> {
    let charged = crate::type_computations::charge_term(term, fuel)
        .map_err(crate::NominalAdmissionError::from);
    layout_admission(charged, fuel)
}

fn nominal_domain_scope(
    ty: &Term,
    prior: &[crate::DeclarationParameter],
    fuel: &mut usize,
) -> bool {
    if crate::type_computations::charge_term(ty, fuel).is_err() {
        return false;
    }
    let mut pending = vec![ty];
    while let Some(term) = pending.pop() {
        match term {
            Term::Rigid { declaration, sort }
                if prior
                    .iter()
                    .any(|p| p.declaration == *declaration && p.sort == *sort) => {}
            Term::Rigid { .. } | Term::Hole { .. } | Term::Runtime(_) | Term::Value { .. } => {
                return false;
            }
            Term::Nominal { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::TupleValue(arguments)
            | Term::ClosureContextValue(arguments)
            | Term::TypeListValue(arguments)
            | Term::TypeListConcat(arguments) => pending.extend(arguments),
            Term::Record(fields) | Term::RecordValue(fields) => {
                pending.extend(fields.iter().map(|(_, ty)| ty))
            }
            Term::OptionalValue { ty, value } => {
                pending.push(ty);
                if let Some(value) = value {
                    pending.push(value);
                }
            }

            Term::ErrorValue { ty, label, value } => {
                pending.push(ty);
                if let Some(label) = label {
                    pending.push(label);
                }
                pending.push(value);
            }
            Term::NominalValue { ty, fields, .. } => {
                pending.push(ty);
                pending.extend(fields);
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
            | Term::UnsafeFunction(inner) => pending.push(inner),
            Term::Function { parameters, result } => {
                pending.push(result);
                pending.extend(parameters.iter().map(|p| &p.ty));
            }
            Term::ContextualClosure { context, signature } => {
                pending.push(context);
                pending.push(signature);
            }
            Term::Closure {
                environment,
                signature,
            } => {
                pending.push(signature);
                pending.extend(environment);
            }
            Term::ErrorResult { errors, result } => {
                pending.push(result);
                pending.extend(errors);
            }
            Term::Computation { effects, result } => {
                pending.push(result);
                for effect in effects {
                    pending.extend(&effect.arguments);
                }
            }
            Term::AssociatedType {
                subject, arguments, ..
            } => {
                pending.push(subject);
                pending.extend(arguments);
            }
            _ => {}
        }
    }
    ty.check_scope(&[]).is_ok()
}
