//! Bounded export of invocation data into stable normal terms.
//! Contextual code and stateful representations never become ordinary terms.
use super::*;

impl Value {
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
                Value::TypeList(types)
                | Value::RuntimeFunction {
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

    /// Build an independent term directly from an admitted borrowed value.
    /// The caller retains ownership; storage handles remain ineligible exports.
    pub(super) fn to_scoped_term(
        &self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
        invocation: bool,
    ) -> Result<Term, ErrorKind> {
        fn charge(budget: &mut comptime_runtime::Budget, work: usize) -> Result<(), ErrorKind> {
            budget.consume(work).map_err(|_| ErrorKind::StepsExhausted)
        }
        fn freeze_integer(
            value: &comptime_runtime::Integer,
            budget: &mut comptime_runtime::Budget,
        ) -> Result<Term, ErrorKind> {
            let limbs = value.bit_len().div_ceil(32);
            if limbs > budget.max_limbs {
                return Err(ErrorKind::RepresentationSizeLimit);
            }
            charge(budget, limbs)?;
            Ok(Term::Integer(value.clone()))
        }
        // Keep unrelated scalar temporaries out of recursive aggregate frames.
        // The dispatch retains one shared depth boundary; each selected path
        // charges exactly the same node and payload work as before.
        #[inline(never)]
        fn visit(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            if depth > 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            match value {
                Value::Object(_) => freeze_object(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::List { .. } => freeze_list(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::Optional { .. } => freeze_optional(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::ErrorResult { .. } => freeze_error(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::Newtype { .. } => freeze_newtype(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::Record { .. } | Value::Variant { .. } => freeze_nominal(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::AnonymousRecord(_) => freeze_record(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                Value::Tuple(_) => freeze_tuple(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
                _ => visit_scalar(
                    value,
                    depth,
                    budget,
                    parameters,
                    value_parameters,
                    invocation,
                ),
            }
        }
        #[inline(never)]
        fn freeze_object(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
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
                        let work = if invocation {
                            scoped_type_value_work_with_evidence(
                                ty,
                                &BTreeMap::new(),
                                parameters,
                                value_parameters,
                                |_, _, _| None,
                            )
                        } else {
                            scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                        }
                        .ok_or(ErrorKind::Type)?;
                        charge(budget, work)?;
                        let domain = ty.clone();
                        let value = visit(
                            value,
                            depth + 1,
                            budget,
                            parameters,
                            value_parameters,
                            invocation,
                        )?;
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
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
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    charge(budget, values.len())?;
                    ty.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
                    // The existing frozen Type-list encoding is still canonical.
                    // VM carriers may be ordinary Lists without creating a second
                    // normal-form identity for the same input value.
                    if arguments.as_slice() == [Term::Primitive(Primitive::Type)] {
                        let elements = values
                            .iter()
                            .map(|value| match value {
                                Value::Type(_) | Value::TypeParameter(_) => visit(
                                    value,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                ),
                                _ => Err(ErrorKind::Type),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        return Ok(Term::TypeListValue(elements));
                    }
                    Term::NominalValue {
                        ty: Box::new(ty.clone()),
                        variant: None,
                        fields: values
                            .iter()
                            .map(|value| {
                                visit(
                                    value,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                )
                            })
                            .collect::<Result<_, _>>()?,
                    }
                }
                _ => unreachable!(),
            })
        }

        #[inline(never)]
        fn freeze_optional(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Optional { ty, value } => {
                    if !matches!(ty, Term::Optional(_)) {
                        return Err(ErrorKind::Type);
                    }
                    let work = scoped_type_value_work_with_evidence(
                        ty,
                        &BTreeMap::new(),
                        parameters,
                        value_parameters,
                        |_, _, _| None,
                    )
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    Term::OptionalValue {
                        ty: Box::new(ty.clone()),
                        value: value
                            .as_ref()
                            .map(|value| {
                                visit(
                                    value,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                )
                                .map(Box::new)
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::ErrorResult { ty, error, value } => {
                    let Term::ErrorResult { errors, result: _ } = ty else {
                        return Err(ErrorKind::Type);
                    };
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    if errors.is_empty() {
                        return Err(ErrorKind::Type);
                    }
                    ty.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
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
                        value: Box::new(visit(
                            value,
                            depth + 1,
                            budget,
                            parameters,
                            value_parameters,
                            invocation,
                        )?),
                    };
                    let work = scoped_term_value_work_with_evidence(
                        &frozen,
                        &BTreeMap::new(),
                        parameters,
                        value_parameters,
                        |_, _, _| None,
                    )
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Newtype { ty, value } => {
                    if !matches!(ty, Term::Nominal { .. }) {
                        return Err(ErrorKind::Type);
                    }
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work.saturating_add(1))?;
                    Term::NominalValue {
                        ty: Box::new(ty.clone()),
                        variant: None,
                        fields: vec![visit(
                            value,
                            depth + 1,
                            budget,
                            parameters,
                            value_parameters,
                            invocation,
                        )?],
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
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
                        let work = if invocation {
                            scoped_term_value_work_with_evidence(
                                argument,
                                &BTreeMap::new(),
                                parameters,
                                value_parameters,
                                |_, _, _| None,
                            )
                        } else {
                            scoped_term_value_work_with_layouts(
                                argument,
                                &BTreeMap::new(),
                                parameters,
                                |_, _, _| None,
                            )
                        }
                        .ok_or(ErrorKind::Type)?;
                        charge(budget, work)?;
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
                            .map(|field| {
                                visit(
                                    field,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                )
                            })
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
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
                                Ok((
                                    name.clone(),
                                    visit(
                                        value,
                                        depth + 1,
                                        budget,
                                        parameters,
                                        value_parameters,
                                        invocation,
                                    )?,
                                ))
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            charge(budget, 1)?;
            Ok(match value {
                Value::Tuple(values) => Term::TupleValue(
                    values
                        .iter()
                        .map(|value| {
                            visit(
                                value,
                                depth + 1,
                                budget,
                                parameters,
                                value_parameters,
                                invocation,
                            )
                        })
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
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            if depth > 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            let instance = match value {
                Value::Function(declaration) => Some((*declaration, &[][..])),
                Value::RuntimeFunction {
                    declaration,
                    arguments,
                } => Some((*declaration, arguments.as_slice())),
                Value::Closure(closure)
                    if closure.environment.is_empty() && closure.capture_order.is_empty() =>
                {
                    closure.declaration_instance()
                }
                _ => None,
            };
            if invocation {
                if let Some((declaration, arguments)) = instance {
                    budget.consume(1).map_err(|_| ErrorKind::StepsExhausted)?;
                    for argument in arguments {
                        nir::type_computations::charge_term(argument, &mut budget.steps)
                            .map_err(|_| ErrorKind::StepsExhausted)?;
                        argument.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
                        // A well-scoped value argument may still contain an
                        // unresolved rigid/hole or pending list operation.
                        // Frozen invocation identities must contain completed
                        // arguments, regardless of their semantic sort.
                        let work = scoped_term_value_work_with_layouts(
                            argument,
                            &BTreeMap::new(),
                            parameters,
                            |_, _, _| None,
                        )
                        .ok_or(ErrorKind::Type)?;
                        budget
                            .consume(work)
                            .map_err(|_| ErrorKind::StepsExhausted)?;
                    }
                    return Ok(Term::FunctionValue {
                        declaration,
                        arguments: arguments.to_vec(),
                    });
                }
            }
            charge(budget, 1)?;
            Ok(match value {
                Value::AbstractValue { declaration, ty } => {
                    if parameters.contains(declaration)
                        || *ty == Term::Primitive(Primitive::Type)
                        || value_parameters.get(declaration) != Some(ty)
                    {
                        return Err(ErrorKind::AbstractValueUnresolved);
                    }
                    let work = scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                        .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    Term::Rigid {
                        declaration: *declaration,
                        sort: nir::Sort::ComptimeValue,
                    }
                }
                Value::TypeParameter(declaration) if parameters.contains(declaration) => {
                    Term::Rigid {
                        declaration: *declaration,
                        sort: nir::Sort::Type,
                    }
                }
                Value::Type(ty) => {
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    ty.clone()
                }

                Value::TypeList(types) => {
                    // Charge width before traversing or allocating a frozen list.
                    charge(budget, types.len())?;
                    for ty in types {
                        let work = if invocation {
                            scoped_type_value_work_with_evidence(
                                ty,
                                &BTreeMap::new(),
                                parameters,
                                value_parameters,
                                |_, _, _| None,
                            )
                        } else {
                            scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                        }
                        .ok_or(ErrorKind::Type)?;
                        charge(budget, work)?;
                    }
                    Term::TypeListValue(types.clone())
                }
                Value::Unit => Term::Unit,
                Value::Bool(value) => Term::Bool(*value),
                Value::Char(value) => Term::Char(*value),
                Value::Float16(bits) => Term::Float16(*bits),
                Value::BFloat16(bits) => Term::BFloat16(*bits),
                Value::Float32(bits) => Term::Float32(*bits),
                Value::Float64(bits) => Term::Float64(*bits),
                Value::BitVec(value) => {
                    if value.words().len() > budget.max_limbs {
                        return Err(ErrorKind::RepresentationSizeLimit);
                    }
                    charge(budget, value.words().len())?;
                    Term::BitVec(value.clone())
                }
                Value::Symbol(value) => {
                    charge(budget, value.len())?;
                    Term::Symbol(value.clone())
                }
                Value::String(value) => {
                    charge(budget, value.len())?;
                    Term::String(value.clone())
                }
                Value::UnsignedInteger(value) => {
                    freeze_integer(&comptime_runtime::Integer::from_u128(*value), budget)?
                }
                Value::SignedInteger(value) => {
                    freeze_integer(&comptime_runtime::Integer::from_i128(*value), budget)?
                }
                Value::Float(value) => {
                    let exponent_limbs = value.exponent().bit_len().div_ceil(32);
                    if exponent_limbs > budget.max_limbs {
                        return Err(ErrorKind::RepresentationSizeLimit);
                    }
                    charge(budget, value.digits().len().saturating_add(exponent_limbs))?;
                    comptime_runtime::Integer::parse_literal(value.digits(), budget).map_err(
                        |error| match error {
                            comptime_runtime::Error::SizeLimit => {
                                ErrorKind::RepresentationSizeLimit
                            }
                            comptime_runtime::Error::BudgetExhausted => ErrorKind::StepsExhausted,
                            _ => ErrorKind::Type,
                        },
                    )?;
                    Term::Float(value.clone())
                }
                Value::Integer(value) => freeze_integer(value, budget)?,

                _ => return Err(ErrorKind::Unsupported),
            })
        }
        visit(self, 0, budget, parameters, value_parameters, invocation)
    }
}

#[cfg(test)]
mod borrowed_input_tests {
    use super::*;
    const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };

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
                .bind_invocation_inputs(&[binding], vec![argument], &signature, &mut env, SOURCE)
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
