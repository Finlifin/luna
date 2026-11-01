//! Intersection of structural evidence across ordinary control-flow paths.
use super::*;
use std::sync::Arc;

impl PartialStagedValue {
    pub(super) fn common(
        left: &Arc<Self>,
        right: &Arc<Self>,
        meta_list: Option<DeclarationId>,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<Arc<Self>, String> {
        if depth >= 128 {
            return Ok(Self::unknown());
        }
        left.charge(fuel)?;
        right.charge(fuel)?;
        // Equality of partial evidence preserves unknown leaves as unknown;
        // it never asserts that two unknown runtime values are equal.
        if left == right {
            return Ok(left.clone());
        }
        let Some((left_shape, left_fields)) = left.aggregate(meta_list) else {
            return Ok(Self::unknown());
        };
        let Some((right_shape, right_fields)) = right.aggregate(meta_list) else {
            return Ok(Self::unknown());
        };
        let shape = match (&left_shape, &right_shape) {
            (
                PartialStagedShape::List { ty: left, exact: a },
                PartialStagedShape::List {
                    ty: right,
                    exact: b,
                },
            ) if left == right => PartialStagedShape::List {
                ty: left.clone(),
                exact: *a && *b && left_fields.len() == right_fields.len(),
            },
            _ if left_shape == right_shape => left_shape.clone(),
            _ => return Ok(Self::unknown()),
        };
        // Unequal list lengths lose exactness but retain their common prefix.
        // Other aggregate layouts still require equal arity.
        if left_fields.len() != right_fields.len()
            && !matches!(left_shape, PartialStagedShape::List { .. })
        {
            return Ok(Self::unknown());
        }
        let mut fields = Vec::with_capacity(left_fields.len().min(right_fields.len()));
        let mut height = 1;
        for (a, b) in left_fields.iter().zip(&right_fields) {
            let field = Self::common(a, b, meta_list, depth + 1, fuel)?;
            height = height.max(field.depth + 1);
            fields.push(field);
        }
        if height >= 128 {
            return Ok(Self::unknown());
        }
        Ok(Arc::new(Self {
            depth: height,
            data: PartialStagedData::Aggregate(shape, fields),
        }))
    }

    pub(super) fn concat(
        left: &Arc<Self>,
        right: &Arc<Self>,
        ty: &Term,
        meta_list: Option<DeclarationId>,
        fuel: &mut usize,
    ) -> Result<Arc<Self>, String> {
        left.charge(fuel)?;
        right.charge(fuel)?;
        let Some((PartialStagedShape::List { ty: left_ty, exact }, mut fields)) =
            left.aggregate(meta_list)
        else {
            return Ok(Self::unknown());
        };
        if left_ty != *ty {
            return Ok(Self::unknown());
        }
        // A prefix does not locate the right operand. Preserve only that prefix.
        let mut complete = false;
        if exact {
            if let Some((
                PartialStagedShape::List {
                    ty: right_ty,
                    exact,
                },
                right_fields,
            )) = right.aggregate(meta_list)
            {
                if right_ty != *ty {
                    return Ok(Self::unknown());
                }
                complete = exact;
                fields.extend(right_fields);
            }
        }
        let depth = fields
            .iter()
            .map(|field| field.depth + 1)
            .max()
            .unwrap_or(1);
        if depth >= 128 {
            return Ok(Self::unknown());
        }
        Ok(Arc::new(Self {
            depth,
            data: PartialStagedData::Aggregate(
                PartialStagedShape::List {
                    ty: left_ty,
                    exact: complete,
                },
                fields,
            ),
        }))
    }

    pub(super) fn list_length(
        &self,
        meta_list: Option<DeclarationId>,
        fuel: &mut usize,
    ) -> Result<Arc<Self>, String> {
        self.charge(fuel)?;
        match self.aggregate(meta_list) {
            Some((PartialStagedShape::List { exact: true, .. }, fields)) => Ok(Self::known(
                Term::Integer(comptime_runtime::Integer::from_u128(fields.len() as u128)),
            )),
            _ => Ok(Self::unknown()),
        }
    }

    // The caller charges the complete evidence before cloning. Decompose only
    // for observation: fully known constants retain their normal representation.
    fn aggregate(
        &self,
        meta_list: Option<DeclarationId>,
    ) -> Option<(PartialStagedShape, Vec<Arc<Self>>)> {
        let known_fields = |fields: &[Term]| fields.iter().cloned().map(Self::known).collect();
        Some(match &self.data {
            PartialStagedData::Aggregate(shape, fields) => (shape.clone(), fields.clone()),
            PartialStagedData::Known(Term::NominalValue {
                ty,
                variant: None,
                fields,
            }) if matches!(ty.as_ref(), Term::NormalForm { head, sort: Sort::Type, arguments }
                if Some(*head) == meta_list && matches!(arguments.as_slice(), [element] if element.sort() == Sort::Type)) =>
            {
                (
                    PartialStagedShape::List {
                        ty: ty.as_ref().clone(),
                        exact: true,
                    },
                    known_fields(fields),
                )
            }
            PartialStagedData::Known(Term::TupleValue(fields)) => {
                (PartialStagedShape::Tuple, known_fields(fields))
            }
            PartialStagedData::Known(Term::RecordValue(fields)) => (
                PartialStagedShape::AnonymousRecord(
                    fields.iter().map(|(name, _)| name.clone()).collect(),
                ),
                fields
                    .iter()
                    .map(|(_, value)| Self::known(value.clone()))
                    .collect(),
            ),
            PartialStagedData::Known(Term::NominalValue {
                ty,
                variant,
                fields,
            }) => {
                let Term::Nominal { declaration, .. } = ty.as_ref() else {
                    return None;
                };
                let shape = match variant {
                    Some(index) => {
                        PartialStagedShape::Variant(*declaration, usize::try_from(*index).ok()?)
                    }
                    None => PartialStagedShape::Record(*declaration),
                };
                (shape, known_fields(fields))
            }
            _ => return None,
        })
    }
}

/// An observation-only lexical environment. No VM storage or inference state is
/// copied; known terms are supplied only to the existing structural inspector.
#[derive(Clone, Default)]
pub(super) struct ObservationScope {
    pub(super) values: BTreeMap<n::LocalId, Arc<PartialStagedValue>>,
    pub(super) known: BTreeMap<n::LocalId, Arc<Term>>,
}
impl ObservationScope {
    pub(super) fn fork(&self, fuel: &mut usize) -> Result<Self, String> {
        *fuel = fuel
            .checked_sub(self.values.len().saturating_add(self.known.len()))
            .ok_or("partial block observation budget exhausted")?;
        Ok(self.clone())
    }

    pub(super) fn insert(
        &mut self,
        binding: n::LocalId,
        value: Arc<PartialStagedValue>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("partial block observation budget exhausted")?;
        if let PartialStagedData::Known(term) = &value.data {
            nir::type_computations::charge_term(term, fuel).map_err(|e| e.to_string())?;
            self.known.insert(binding, Arc::new(term.clone()));
        } else {
            self.known.remove(&binding);
        }
        self.values.insert(binding, value);
        Ok(())
    }
}

impl Checker<'_, '_> {
    /// Inspect a checked body using immutable argument evidence. The call stays
    /// in executable IR, including argument evaluation and failing contracts.
    pub(super) fn partial_staged_call(
        &self,
        callee: &n::Expr,
        arguments: &[n::CallArgument],
        depth: usize,
        fuel: &mut usize,
        caller: &ObservationScope,
    ) -> Result<Option<Arc<PartialStagedValue>>, String> {
        let (signature, bindings, body) = match &callee.kind {
            K::Function(id)
            | K::FunctionInstance {
                declaration: id,
                arguments: _,
            } if !matches!(&callee.kind, K::FunctionInstance { arguments, .. } if !arguments.is_empty()) =>
            {
                *fuel = fuel
                    .checked_sub(
                        self.generated_functions.len() + self.ordinary_behavior_functions.len(),
                    )
                    .ok_or("partial callable lookup budget exhausted")?;
                let Some(function) = self
                    .generated_functions
                    .iter()
                    .chain(&self.ordinary_behavior_functions)
                    .find(|function| function.declaration == *id)
                else {
                    return Ok(None);
                };
                if function.phase != n::FunctionPhase::Comptime
                    || !function.type_parameters.is_empty()
                {
                    return Ok(None);
                }
                (
                    &function.signature,
                    function.parameters.as_slice(),
                    &function.body,
                )
            }
            K::Closure {
                parameters,
                captures,
                body,
            } if captures.is_empty() && callee.ty.is_comptime_callable() => (
                callee.ty.callable_signature(),
                parameters.as_slice(),
                body.as_ref(),
            ),
            _ => return Ok(None),
        };
        let Term::Function { parameters, .. } = signature.callable_signature() else {
            return Ok(None);
        };
        if parameters.len() != arguments.len() || bindings.len() != parameters.len() {
            return Ok(None);
        }
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or("partial callable input budget exhausted")?;
        let mut inputs = vec![None; parameters.len()];
        let mut observed = vec![None; parameters.len()];
        for argument in arguments {
            let index = argument.parameter;
            if index >= parameters.len() || observed[index].is_some() || argument.is_default {
                return Ok(None);
            }
            let value = self.partial_staged_value_in(&argument.value, depth + 1, fuel, caller)?;
            if parameters[index].sort != Sort::Runtime {
                let PartialStagedData::Known(term) = &value.data else {
                    return Ok(None);
                };
                nir::type_computations::charge_term(term, fuel)
                    .map_err(|error| error.to_string())?;
                inputs[index] = Some(term.clone());
            }
            observed[index] = Some(value);
        }
        nir::type_computations::charge_term(signature, fuel).map_err(|error| error.to_string())?;
        let binding = signature
            .prepare_function_inputs(&inputs)
            .map_err(|error| error.to_string())?;
        let mut copy_fuel = *fuel as u64;
        let copied = nir::copy_budget::charge_expression(body, &mut copy_fuel);
        *fuel = copy_fuel as usize;
        copied.map_err(|_| "partial callable body budget exhausted")?;
        let mut body = body.clone();
        body.rewrite_types(&mut |ty| {
            nir::type_computations::charge_term(ty, fuel).map_err(|error| error.to_string())?;
            binding.rebind(ty).map_err(|error| error.to_string())
        })?;
        let mut scope = ObservationScope::default();
        for (local, value) in bindings.iter().zip(observed) {
            scope.insert(*local, value.unwrap(), fuel)?;
        }
        self.partial_staged_value_in(&body, depth + 1, fuel, &scope)
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_observation_budget_exhaustion_does_not_mutate_or_poison_evidence() {
        let head = DeclarationId {
            package: 1,
            local: 1,
        };
        let ty = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![Term::Primitive(nir::Primitive::Type)],
        };
        let list = |fields, exact| {
            Arc::new(PartialStagedValue {
                depth: 1,
                data: PartialStagedData::Aggregate(
                    PartialStagedShape::List {
                        ty: ty.clone(),
                        exact,
                    },
                    fields,
                ),
            })
        };
        let left = list(
            vec![
                PartialStagedValue::known(Term::Primitive(nir::Primitive::Bool)),
                PartialStagedValue::unknown(),
            ],
            true,
        );
        let right = list(
            vec![PartialStagedValue::known(Term::Primitive(
                nir::Primitive::U8,
            ))],
            true,
        );
        let before = [left.to_string(), right.to_string()];
        for operation in [0, 1, 2] {
            let run = |fuel: &mut usize| match operation {
                0 => PartialStagedValue::concat(&left, &right, &ty, Some(head), fuel),
                1 => PartialStagedValue::common(&left, &right, Some(head), 0, fuel),
                _ => left.list_length(Some(head), fuel),
            };
            let mut generous = 100_000;
            let expected = run(&mut generous).unwrap().to_string();
            let cost = 100_000 - generous;
            assert!(cost > 0);
            for budget in 0..=cost {
                let mut fuel = budget;
                let result = run(&mut fuel);
                assert_eq!(
                    result.is_ok(),
                    budget == cost,
                    "operation {operation}, budget {budget}"
                );
                assert!(fuel <= budget);
                assert_eq!([left.to_string(), right.to_string()], before);
                let mut fresh = cost;
                assert_eq!(run(&mut fresh).unwrap().to_string(), expected);
                assert_eq!(fresh, 0);
            }
        }
    }

    #[test]
    fn list_observation_dump_distinguishes_empty_prefix_from_empty_list() {
        let ty = Term::NormalForm {
            head: DeclarationId {
                package: 1,
                local: 1,
            },
            sort: Sort::Type,
            arguments: vec![Term::Primitive(nir::Primitive::Type)],
        };
        for exact in [false, true] {
            let evidence = PartialStagedValue {
                depth: 1,
                data: PartialStagedData::Aggregate(
                    PartialStagedShape::List {
                        ty: ty.clone(),
                        exact,
                    },
                    vec![],
                ),
            };
            let dump = evidence.to_string();
            assert!(dump.starts_with("(list (type ("));
            assert!(dump.contains(&format!("(exact {exact})")));
            let mut balance = 0;
            for ch in dump.chars() {
                match ch {
                    '(' => balance += 1,
                    ')' => balance -= 1,
                    _ => {}
                }
                assert!(balance >= 0, "{dump}");
            }
            assert_eq!(balance, 0, "{dump}");
            let mut fuel = 1000;
            let length = evidence
                .list_length(
                    Some(DeclarationId {
                        package: 1,
                        local: 1,
                    }),
                    &mut fuel,
                )
                .unwrap();
            if exact {
                assert!(
                    matches!(&length.data,PartialStagedData::Known(Term::Integer(value)) if *value == comptime_runtime::Integer::from_u128(0))
                );
            } else {
                assert!(matches!(&length.data, PartialStagedData::Unknown));
            }
        }
    }
}
