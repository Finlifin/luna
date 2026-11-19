//! Bounded export of invocation data into stable normal terms.
//! Contextual code and stateful representations never become ordinary terms.
use super::*;

/// Binder evidence available while converting live data to an immutable term.
struct ExportScope<'a> {
    type_parameters: &'a BTreeSet<DeclarationId>,
    value_parameters: &'a BTreeMap<DeclarationId, Term>,
    invocation: bool,
}

impl ExportScope<'_> {
    fn inspect_type(
        &self,
        ty: &Term,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<(), ErrorKind> {
        if ty.sort() != nir::Sort::Type {
            return Err(ErrorKind::Type);
        }
        self.inspect_term(ty, budget)
    }

    fn inspect_term(
        &self,
        term: &Term,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<(), ErrorKind> {
        type_descriptions::inspect_data_term(
            term,
            self.type_parameters,
            self.value_parameters,
            &mut budget.steps,
        )
        .map(|_| ())
    }
}

/// Check completion and scope of an already domain-admitted immutable input.
/// Type and value inputs retain their semantic sort. This checks representation,
/// not the declaration's parameter domains or permission to invoke a callable.
pub fn validate_snapshot_term(
    term: &Term,
    budget: &mut comptime_runtime::Budget,
    parameters: &BTreeSet<DeclarationId>,
) -> Result<(), ErrorKind> {
    ExportScope {
        type_parameters: parameters,
        value_parameters: &BTreeMap::new(),
        invocation: false,
    }
    .inspect_term(term, budget)
}

impl Value {
    /// Inspect a scalar for a capture or normal input without allocating its
    /// exported representation. The receiving domain is checked separately.
    pub fn check_scalar_snapshot(
        &self,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<(), ErrorKind> {
        use comptime_runtime::Error as RuntimeError;
        let runtime_error = |error| match error {
            RuntimeError::SizeLimit => ErrorKind::RepresentationSizeLimit,
            RuntimeError::BudgetExhausted => ErrorKind::StepsExhausted,
            _ => ErrorKind::Type,
        };
        budget.consume(1).map_err(runtime_error)?;
        let limbs = match self {
            Self::Integer(value) => value.bit_len().div_ceil(32),
            Self::UnsignedInteger(value) => {
                (u128::BITS - value.leading_zeros()).div_ceil(32) as usize
            }
            Self::SignedInteger(value) => {
                (u128::BITS - value.unsigned_abs().leading_zeros()).div_ceil(32) as usize
            }
            Self::BitVec(bits) => bits.words().len(),
            Self::Float(value) => {
                let exponent = value.exponent().bit_len().div_ceil(32);
                if exponent > budget.max_limbs {
                    return Err(ErrorKind::RepresentationSizeLimit);
                }
                budget
                    .consume(value.digits().len().saturating_add(exponent))
                    .map_err(runtime_error)?;
                return value.check_representation(budget).map_err(runtime_error);
            }
            Self::String(value) | Self::Symbol(value) => {
                return budget.consume(value.len()).map_err(runtime_error);
            }
            Self::Unit
            | Self::Bool(_)
            | Self::Char(_)
            | Self::Float16(_)
            | Self::BFloat16(_)
            | Self::Float32(_)
            | Self::Float64(_) => return Ok(()),
            _ => return Err(ErrorKind::Unsupported),
        };
        if limbs > budget.max_limbs {
            return Err(ErrorKind::RepresentationSizeLimit);
        }
        budget.consume(limbs).map_err(runtime_error)
    }

    fn freeze_scalar_term(&self, budget: &mut comptime_runtime::Budget) -> Result<Term, ErrorKind> {
        self.check_scalar_snapshot(budget)?;
        Ok(match self {
            Self::Unit => Term::Unit,
            Self::Bool(value) => Term::Bool(*value),
            Self::Char(value) => Term::Char(*value),
            Self::Float16(bits) => Term::Float16(*bits),
            Self::BFloat16(bits) => Term::BFloat16(*bits),
            Self::Float32(bits) => Term::Float32(*bits),
            Self::Float64(bits) => Term::Float64(*bits),
            Self::String(text) => Term::String(text.clone()),
            Self::Symbol(text) => Term::Symbol(text.clone()),
            Self::BitVec(bits) => Term::BitVec(bits.clone()),
            Self::Float(value) => Term::Float(value.clone()),
            Self::Integer(value) => Term::Integer(value.clone()),
            Self::SignedInteger(value) => {
                Term::Integer(comptime_runtime::Integer::from_i128(*value))
            }
            Self::UnsignedInteger(value) => {
                Term::Integer(comptime_runtime::Integer::from_u128(*value))
            }
            _ => return Err(ErrorKind::Unsupported),
        })
    }

    pub(super) fn into_scoped_term(
        self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
        invocation: bool,
    ) -> Result<Term, ErrorKind> {
        let result = self.to_scoped_term(budget, parameters, value_parameters, invocation);
        // Even rejected host data must be released without recursively dropping
        // a potentially unbounded aggregate tail. Cleanup is not fuel-limited.
        let mut pending = vec![self];
        while let Some(value) = pending.pop() {
            match value {
                Value::Tuple(values) => pending.extend(values),
                Value::List { ty, values } => {
                    ty.drop_iteratively();
                    pending.extend(values);
                }
                Value::Record {
                    fields, arguments, ..
                }
                | Value::Variant {
                    fields, arguments, ..
                } => {
                    pending.extend(fields);
                    for argument in arguments {
                        argument.drop_iteratively();
                    }
                }
                Value::AnonymousRecord(fields) => {
                    pending.extend(fields.into_iter().map(|(_, value)| value))
                }
                Value::Object(object) => {
                    for entry in object.into_entries() {
                        let typed = entry.into_value();
                        typed.ty.drop_iteratively();
                        pending.push(typed.value);
                    }
                }
                Value::Type(ty) => ty.drop_iteratively(),
                Value::Optional { ty, value } => {
                    ty.drop_iteratively();
                    if let Some(value) = value {
                        pending.push(*value);
                    }
                }
                Value::ErrorResult { ty, value, .. } | Value::Newtype { ty, value } => {
                    ty.drop_iteratively();
                    pending.push(*value);
                }
                Value::FunctionInstance {
                    arguments: types, ..
                } => {
                    for ty in types {
                        ty.drop_iteratively();
                    }
                }
                Value::RuntimeTraitMethod(method) => {
                    method.subject.drop_iteratively();
                    for ty in method
                        .interface
                        .arguments
                        .into_iter()
                        .chain(method.method_arguments)
                    {
                        ty.drop_iteratively();
                    }
                }
                _ => {}
            }
        }
        result
    }

    /// Freeze borrowed immutable data under a closed export scope.
    pub fn to_normal_term(&self, budget: &mut comptime_runtime::Budget) -> Result<Term, ErrorKind> {
        self.to_scoped_term(budget, &BTreeSet::new(), &BTreeMap::new(), false)
    }

    /// Build an independent term directly from an admitted borrowed value.
    /// The caller retains ownership; storage handles remain ineligible exports.
    pub(super) fn to_scoped_term(
        &self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
        invocation: bool,
    ) -> Result<Term, ErrorKind> {
        self.freeze_input_template(budget, parameters, value_parameters, invocation, None)
    }

    pub(super) fn freeze_input_template(
        &self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
        invocation: bool,
        mut holes: Option<&mut Vec<nir::program::DescriptionLiteral>>,
    ) -> Result<Term, ErrorKind> {
        fn charge(budget: &mut comptime_runtime::Budget, work: usize) -> Result<(), ErrorKind> {
            budget.consume(work).map_err(|_| ErrorKind::StepsExhausted)
        }
        // Keep unrelated scalar temporaries out of recursive aggregate frames.
        // The dispatch retains one shared depth boundary; each selected path
        // charges its node and payload work to the export budget.
        #[inline(never)]
        fn visit(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            if depth > 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            if let Value::Description(data) = value {
                charge(budget, 1)?;
                let holes = holes.as_deref_mut().ok_or(ErrorKind::Unsupported)?;
                let index = holes.len();
                holes.push(data.clone());
                return Ok(Term::Bound {
                    depth: 0,
                    index,
                    sort: nir::Sort::Type,
                });
            }
            match value {
                Value::Object(_) => freeze_object(value, depth, budget, scope, holes),
                Value::List { .. } => freeze_list(value, depth, budget, scope, holes),
                Value::Optional { .. } => freeze_optional(value, depth, budget, scope, holes),
                Value::ErrorResult { .. } => freeze_error(value, depth, budget, scope, holes),
                Value::Newtype { .. } => freeze_newtype(value, depth, budget, scope, holes),
                Value::Record { .. } | Value::Variant { .. } => {
                    freeze_nominal(value, depth, budget, scope, holes)
                }
                Value::AnonymousRecord(_) => freeze_record(value, depth, budget, scope, holes),
                Value::Tuple(_) => freeze_tuple(value, depth, budget, scope, holes),
                _ => visit_scalar(value, depth, budget, scope),
            }
        }
        #[inline(never)]
        fn freeze_object(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Object(object) => {
                    charge(budget, object.entries().len().saturating_mul(4))?;
                    let mut entries = Vec::with_capacity(object.entries().len());
                    for entry in object.entries() {
                        if let Some(key) = entry.key() {
                            charge(budget, key.len())?;
                        }
                        let ObjectValue { ty, value } = entry.value();
                        scope.inspect_type(ty, budget)?;
                        let domain = ty.clone();
                        let value = visit(value, depth + 1, budget, scope, holes)?;
                        entries.push(nir::objects::entry(entry.key(), domain, value));
                    }
                    nir::objects::value(entries)
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_list(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::List { ty, values } => {
                    let Term::NormalForm {
                        sort: nir::Sort::Type,
                        arguments,
                        ..
                    } = ty
                    else {
                        return Err(ErrorKind::Type);
                    };
                    if !matches!(arguments.as_slice(), [element]
                        if element.sort() == nir::Sort::Type)
                    {
                        return Err(ErrorKind::Type);
                    }
                    scope.inspect_type(ty, budget)?;
                    charge(budget, values.len())?;
                    let fields = values
                        .iter()
                        .map(|value| visit(value, depth + 1, budget, scope, holes))
                        .collect::<Result<Vec<_>, _>>()?;
                    nir::comptime_values::freeze_admitted_list(ty, fields, &mut budget.steps)
                        .map_err(|error| match error {
                            nir::comptime_values::Error::Budget => ErrorKind::StepsExhausted,
                            nir::comptime_values::Error::Invalid(_) => ErrorKind::Type,
                        })?
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_optional(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Optional { ty, value } => {
                    if !matches!(ty, Term::Optional(_)) {
                        return Err(ErrorKind::Type);
                    }
                    scope.inspect_type(ty, budget)?;
                    Term::OptionalValue {
                        ty: Box::new(ty.clone()),
                        value: value
                            .as_ref()
                            .map(|value| {
                                visit(value, depth + 1, budget, scope, holes).map(Box::new)
                            })
                            .transpose()?,
                    }
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_error(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::ErrorResult { ty, error, value } => {
                    let Term::ErrorResult { errors, result: _ } = ty else {
                        return Err(ErrorKind::Type);
                    };
                    scope.inspect_type(ty, budget)?;
                    if errors.is_empty() {
                        return Err(ErrorKind::Type);
                    }
                    let label = error
                        .map(|index| {
                            errors
                                .get(index)
                                .cloned()
                                .map(Box::new)
                                .ok_or(ErrorKind::Type)
                        })
                        .transpose()?;
                    let frozen = Term::ErrorValue {
                        ty: Box::new(ty.clone()),
                        label,
                        value: Box::new(visit(value, depth + 1, budget, scope, holes)?),
                    };
                    scope.inspect_term(&frozen, budget)?;
                    frozen
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_newtype(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Newtype { ty, value } => {
                    if !matches!(ty, Term::Nominal { .. }) {
                        return Err(ErrorKind::Type);
                    }
                    scope.inspect_type(ty, budget)?;
                    charge(budget, 1)?;
                    Term::NominalValue {
                        ty: Box::new(ty.clone()),
                        variant: None,
                        fields: vec![visit(value, depth + 1, budget, scope, holes)?],
                    }
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_nominal(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Record {
                    declaration,
                    arguments,
                    fields,
                }
                | Value::Variant {
                    declaration,
                    arguments,
                    fields,
                    ..
                } => {
                    charge(budget, arguments.len().saturating_add(fields.len()))?;
                    for argument in arguments {
                        scope.inspect_term(argument, budget)?;
                    }
                    let ty = Term::Nominal {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    };
                    Term::NominalValue {
                        ty: Box::new(ty),
                        variant: match value {
                            Value::Variant { index, .. } => Some(
                                u32::try_from(*index)
                                    .map_err(|_| ErrorKind::RepresentationSizeLimit)?,
                            ),
                            _ => None,
                        },
                        fields: fields
                            .iter()
                            .map(|field| visit(field, depth + 1, budget, scope, holes))
                            .collect::<Result<_, _>>()?,
                    }
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_record(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::AnonymousRecord(fields) => {
                    let mut names = std::collections::BTreeSet::new();
                    Term::RecordValue(
                        fields
                            .iter()
                            .map(|(name, value)| {
                                charge(budget, name.len())?;
                                if !names.insert(name.clone()) {
                                    return Err(ErrorKind::Type);
                                }
                                Ok((name.clone(), visit(value, depth + 1, budget, scope, holes)?))
                            })
                            .collect::<Result<_, _>>()?,
                    )
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_tuple(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
            holes: &mut Option<&mut Vec<nir::program::DescriptionLiteral>>,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Tuple(values) => Term::TupleValue(
                    values
                        .iter()
                        .map(|value| visit(value, depth + 1, budget, scope, holes))
                        .collect::<Result<_, _>>()?,
                ),
                _ => unreachable!(),
            })
        }
        #[inline(never)]
        fn visit_scalar(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            scope: &ExportScope<'_>,
        ) -> Result<Term, ErrorKind> {
            if depth > 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            if let Value::RuntimeTraitMethod(method) = value {
                value.validate_callable_identity_snapshot(budget, scope.type_parameters)?;
                return Ok(Term::MethodValue(method.clone()));
            }
            if scope.invocation {
                if let Value::Function(declaration) = value {
                    charge(budget, 1)?;
                    return Ok(Term::FunctionValue {
                        declaration: *declaration,
                        arguments: vec![],
                    });
                }
                if let Some((declaration, arguments)) = value.declaration_instance() {
                    value.validate_callable_identity_snapshot(budget, scope.type_parameters)?;
                    return Ok(Term::FunctionValue {
                        declaration,
                        arguments: arguments.to_vec(),
                    });
                }
            }
            Ok(match value {
                Value::AbstractValue { declaration, ty } => {
                    charge(budget, 1)?;
                    if scope.type_parameters.contains(declaration)
                        || *ty == Term::Primitive(Primitive::Type)
                        || scope.value_parameters.get(declaration) != Some(ty)
                    {
                        return Err(ErrorKind::AbstractValueUnresolved);
                    }
                    // The binder's domain can depend on earlier opaque
                    // values. Check its structural scope in the same prefix.
                    scope.inspect_type(ty, budget)?;
                    Term::Rigid {
                        declaration: *declaration,
                        sort: nir::Sort::ComptimeValue,
                    }
                }
                Value::TypeParameter(declaration)
                    if scope.type_parameters.contains(declaration) =>
                {
                    charge(budget, 1)?;
                    Term::Rigid {
                        declaration: *declaration,
                        sort: nir::Sort::Type,
                    }
                }
                Value::Type(ty) => {
                    charge(budget, 1)?;
                    scope.inspect_type(ty, budget)?;
                    ty.clone()
                }

                scalar => return scalar.freeze_scalar_term(budget),
            })
        }
        let closed_values = BTreeMap::new();
        visit(
            self,
            0,
            budget,
            &ExportScope {
                type_parameters: parameters,
                value_parameters: if invocation {
                    value_parameters
                } else {
                    &closed_values
                },
                invocation,
            },
            &mut holes,
        )
    }
}

#[cfg(test)]
mod borrowed_input_tests {
    use super::*;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };

    #[test]
    fn exact_float_normal_export_reuses_representation_metadata_not_limits() {
        let text = format!("{}.0", "7".repeat(2048));
        let number = comptime_runtime::decimal::DecimalValue::parse_literal(
            &text,
            &mut comptime_runtime::Budget::default(),
        )
        .unwrap();
        let value = Value::Float(number.clone());
        let parameters = BTreeSet::new();
        let domains = BTreeMap::new();
        for invocation in [false, true] {
            let mut small = comptime_runtime::Budget {
                steps: 10000,
                max_limbs: 1,
            };
            assert_eq!(
                value.to_scoped_term(&mut small, &parameters, &domains, invocation),
                Err(ErrorKind::RepresentationSizeLimit)
            );
            let mut linear = comptime_runtime::Budget {
                steps: 4200,
                max_limbs: 4096,
            };
            assert_eq!(
                value
                    .to_scoped_term(&mut linear, &parameters, &domains, invocation)
                    .unwrap(),
                Term::Float(number.clone())
            );
            let mut exhausted = comptime_runtime::Budget {
                steps: 1,
                max_limbs: 4096,
            };
            assert_eq!(
                value.to_scoped_term(&mut exhausted, &parameters, &domains, invocation),
                Err(ErrorKind::StepsExhausted)
            );
        }
        assert_eq!(value, Value::Float(number));
    }

    #[test]
    fn staged_binding_after_ordinary_prefix_preserves_dependent_domains() {
        let package = Package::default();
        let bindings = [0, 1, 2].map(|index| LocalId { owner: 1, index });
        let bool_ty = Term::Primitive(Primitive::Bool);
        let type_input = Term::Bound {
            depth: 0,
            index: 1,
            sort: nir::Sort::Type,
        };
        let signature = Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: bool_ty.clone(),
                },
                nir::Parameter {
                    sort: nir::Sort::Type,
                    ty: Term::Primitive(Primitive::Type),
                },
                nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: type_input.clone(),
                },
            ],
            result: Box::new(type_input.clone()),
        };
        for stored in [false, true] {
            for valid in [false, true] {
                let prefix = if stored {
                    Value::Storage(Storage::new(Value::Bool(true)))
                } else {
                    Value::Bool(true)
                };
                let suffix = if valid {
                    Value::Bool(false)
                } else {
                    Value::Type(bool_ty.clone())
                };
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut env = Environment::new();
                let result = vm.bind_invocation_inputs(
                    &bindings,
                    vec![prefix, Value::Type(bool_ty.clone()), suffix],
                    &signature,
                    &mut env,
                    SOURCE,
                    None,
                );
                if !valid {
                    assert!(result.is_err());
                    assert!(!env.contains_key(&bindings[2]));
                    continue;
                }
                let scope = result.unwrap().unwrap();
                assert_eq!(scope.rebind(&type_input).unwrap(), bool_ty);
                assert_eq!(
                    scope.signature(),
                    &Term::Function {
                        parameters: vec![
                            nir::Parameter {
                                sort: nir::Sort::Runtime,
                                ty: bool_ty.clone(),
                            };
                            2
                        ],
                        result: Box::new(bool_ty.clone()),
                    }
                );
                assert_eq!(env[&bindings[2]], Value::Bool(false));
            }
        }
    }

    #[test]
    fn borrowed_exports_retain_scope_checks_and_leave_input_intact_on_failure() {
        let parameter = DeclarationId {
            package: 1,
            local: 7,
        };
        let value = Value::Tuple(vec![Value::Bool(true), Value::TypeParameter(parameter)]);
        let original = value.clone();
        let parameters = BTreeSet::from([parameter]);
        let mut failures = 0;
        let mut successes = 0;
        for steps in 0..40 {
            let mut borrowed = comptime_runtime::Budget {
                steps,
                max_limbs: 4096,
            };
            let mut owned = comptime_runtime::Budget {
                steps,
                max_limbs: 4096,
            };
            let result = value.to_scoped_term(&mut borrowed, &parameters, &BTreeMap::new(), true);
            assert_eq!(
                result,
                value
                    .clone()
                    .into_scoped_term(&mut owned, &parameters, &BTreeMap::new(), true)
            );
            assert_eq!(borrowed.steps, owned.steps);
            assert_eq!(value, original);
            match result {
                Ok(term) => {
                    successes += 1;
                    assert_eq!(
                        term,
                        Term::TupleValue(vec![
                            Term::Bool(true),
                            Term::Rigid {
                                declaration: parameter,
                                sort: nir::Sort::Type
                            }
                        ])
                    );
                }
                Err(error) => {
                    failures += 1;
                    assert_eq!(error, ErrorKind::StepsExhausted);
                }
            }
        }
        assert!(successes > 0 && failures > 0);
        let mut budget = comptime_runtime::Budget {
            steps: 1000,
            max_limbs: 4096,
        };
        assert!(
            value
                .to_scoped_term(&mut budget, &BTreeSet::new(), &BTreeMap::new(), true)
                .is_err()
        );
        let storage = Value::Storage(Storage::new(Value::Bool(true)));
        assert!(
            storage
                .to_scoped_term(&mut budget, &BTreeSet::new(), &BTreeMap::new(), true)
                .is_err()
        );
        assert_eq!(value, original);
    }

    #[test]
    fn bound_terms_keep_the_original_input_after_owned_and_shared_writes() {
        let package = Package::default();
        let binding = LocalId { owner: 1, index: 0 };
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::ComptimeValue,
                ty: Term::Tuple(vec![Term::Primitive(Primitive::Bool); 2]),
            }],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let old = Value::Tuple(vec![Value::Bool(true), Value::Bool(false)]);
        let new = Value::Tuple(vec![Value::Bool(false), Value::Bool(true)]);
        for carrier in 0..3 {
            let storage = Storage::new(old.clone());
            let argument = match carrier {
                0 => old.clone(),
                1 => Value::Storage(storage.clone()),
                _ => {
                    Value::ComptimeReference(value_reference::ValueReference::root(storage.clone()))
                }
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            let scope = vm
                .bind_invocation_inputs(
                    &[binding],
                    vec![argument],
                    &signature,
                    &mut env,
                    SOURCE,
                    None,
                )
                .unwrap()
                .unwrap();
            vm.write_comptime_place(env.get_mut(&binding).unwrap(), &[], new.clone(), SOURCE)
                .unwrap();
            assert_eq!(
                vm.invocation_slot_value(&env[&binding], Some(SOURCE))
                    .unwrap(),
                new
            );
            assert_eq!(
                scope
                    .rebind(&Term::Bound {
                        depth: 0,
                        index: 0,
                        sort: nir::Sort::ComptimeValue
                    })
                    .unwrap(),
                Term::TupleValue(vec![Term::Bool(true), Term::Bool(false)])
            );
            if carrier != 0 {
                assert_eq!(*storage.0.lock().unwrap(), new);
            }
        }
    }
}
