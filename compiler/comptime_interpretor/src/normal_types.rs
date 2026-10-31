//! Invocation-local bindings and contextual normal-form construction.
use super::*;

/// Substitutions follow completion order; final arguments follow declaration order.
/// Slots store indices rather than cloning potentially large comptime terms.
pub(super) struct Bindings {
    slots: Vec<Option<usize>>,
    pub parameters: Vec<nir::DeclarationParameter>,
    pub arguments: Vec<Term>,
    domains: Vec<Term>,
}
impl Bindings {
    pub fn new(count: usize) -> Self {
        Self {
            slots: vec![None; count],
            parameters: vec![],
            arguments: vec![],
            domains: vec![],
        }
    }
    pub fn instantiate(
        &self,
        vm: &mut Interpreter<'_>,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        vm.integer_operation(Some(source), |budget| budget.consume(self.parameters.len()))?;
        if self
            .parameters
            .iter()
            .zip(&self.arguments)
            .all(|(p, a)| p.sort == a.sort())
        {
            return vm.instantiate_contextual_domain(ty, &self.parameters, &self.arguments, source);
        }
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let ids = self
            .parameters
            .iter()
            .map(|p| p.declaration)
            .collect::<Vec<_>>();
        let result = nir::DeclarationSubstitution::bounded(
            &ids,
            &self.domains,
            &self.arguments,
            &[],
            &mut remaining,
        )
        .and_then(|substitution| substitution.instantiate_bounded(ty, &mut remaining));
        vm.budget.steps -= (before - remaining) as u64;
        result.map_err(|error| {
            vm.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }
    pub fn specialize(
        &self,
        vm: &mut Interpreter<'_>,
        input: &mut nir::program::NormalFormArgument,
    ) -> Result<(), Error> {
        let source = input.source();
        let mut rewrite = |ty: &Term| self.instantiate(vm, ty, source);
        for pattern in input.patterns_mut() {
            pattern.rewrite_types(&mut rewrite)?;
        }
        for expression in input.expressions_mut() {
            expression.rewrite_types(&mut rewrite)?;
        }
        Ok(())
    }
    pub fn contains(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(Option::is_some)
    }
    pub fn finish(self) -> Option<Vec<Term>> {
        if self
            .slots
            .iter()
            .enumerate()
            .all(|(slot, index)| *index == Some(slot))
        {
            return Some(self.arguments);
        }
        let order = self.slots.into_iter().collect::<Option<Vec<_>>>()?;
        let mut terms: Vec<_> = self.arguments.into_iter().map(Some).collect();
        order
            .into_iter()
            .map(|index| terms.get_mut(index)?.take())
            .collect()
    }
}
impl fmt::Display for Bindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(normal-bindings")?;
        for (slot, index) in self.slots.iter().enumerate() {
            write!(f, " (slot {slot}")?;
            if let Some(index) = index {
                write!(
                    f,
                    " (declaration {}) (value {})",
                    self.parameters[*index].declaration, self.arguments[*index]
                )?;
            } else {
                write!(f, " pending")?;
            }
            write!(f, ")")?;
        }
        write!(f, ")")
    }
}
pub(super) struct Normal {
    pub head: DeclarationId,
    pub bindings: Bindings,
}
pub(super) struct Inputs {
    pub normal: Option<Normal>,
    pub prepared: BTreeMap<usize, Prepared>,
}
impl Inputs {
    pub fn new(head: DeclarationId, count: usize) -> Self {
        Self {
            normal: Some(Normal {
                head,
                bindings: Bindings::new(count),
            }),
            prepared: BTreeMap::new(),
        }
    }
    pub fn contextual() -> Self {
        Self {
            normal: None,
            prepared: BTreeMap::new(),
        }
    }
}
impl fmt::Display for Inputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(contextual-inputs")?;
        if let Some(normal) = &self.normal {
            write!(f, " (normal (head {}) {})", normal.head, normal.bindings)?;
        }
        write!(f, " (prepared")?;
        for (slot, value) in &self.prepared {
            write!(f, " (%{slot} {value})")?;
        }
        write!(f, "))")
    }
}
impl Interpreter<'_> {
    pub(super) fn prepare_normal_type(
        &mut self,
        head: DeclarationId,
        ty: &Term,
        count: usize,
        diverges: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        let metadata = self
            .normal_forms
            .get(&head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let expected = if diverges {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if *ty != Term::Primitive(expected) || count != metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.integer_operation(Some(source), |budget| budget.consume(count))
    }
    pub(super) fn prepare_compiled_normal_input(
        &mut self,
        inputs: &mut Inputs,
        index: usize,
        input: &comptime_compiler::normal_types::Shape,
        source: SourceRange,
    ) -> Result<(), Error> {
        let normal = inputs
            .normal
            .as_ref()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let metadata = self
            .normal_forms
            .get(&normal.head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        if index != normal.bindings.arguments.len() || index >= metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let expected = normal
            .bindings
            .instantiate(self, &metadata.parameters[index].ty, source)?;
        inputs.prepared.clear();
        self.prepare_normal_shape(input, &expected, &mut inputs.prepared, 0)
    }
    pub(super) fn normal_list_domain(
        &mut self,
        expected: &Term,
        count: usize,
        source: SourceRange,
    ) -> Result<Term, Error> {
        if count > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        self.integer_operation(Some(source), |budget| {
            let before = budget.steps;
            let mut fuel = before;
            if nir::type_computations::charge_term(expected, &mut fuel).is_err() {
                return budget.consume(before.saturating_add(1));
            }
            budget.consume(before - fuel)
        })?;
        let element = self
            .list_element(expected)
            .cloned()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(count))?;
        Ok(element)
    }
    pub(super) fn prepare_normal_shape(
        &mut self,
        shape: &comptime_compiler::normal_types::Shape,
        expected: &Term,
        prepared: &mut BTreeMap<usize, Prepared>,
        depth: usize,
    ) -> Result<(), Error> {
        use comptime_compiler::normal_types::{Input, ShapeKind};
        let source = shape.source;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        self.tick(Some(source))?;
        let value =
            match &shape.kind {
                ShapeKind::Match(arms) => {
                    for arm in arms {
                        if let Some((ty, guard_source)) = &arm.guard {
                            if !matches!(ty, Term::Primitive(Primitive::Bool | Primitive::Never)) {
                                return Err(self.failure(ErrorKind::Type, Some(*guard_source)));
                            }
                        }
                        self.prepare_normal_shape(&arm.body, expected, prepared, depth + 1)?;
                    }
                    return Ok(());
                }
                ShapeKind::Choice {
                    condition,
                    condition_source,
                    then_input,
                    else_input,
                } => {
                    if !matches!(
                        condition,
                        Term::Primitive(Primitive::Bool | Primitive::Never)
                    ) {
                        return Err(self.failure(ErrorKind::Type, Some(*condition_source)));
                    }
                    self.prepare_normal_shape(then_input, expected, prepared, depth + 1)?;
                    self.prepare_normal_shape(else_input, expected, prepared, depth + 1)?;
                    return Ok(());
                }
                ShapeKind::Block { result } => {
                    if let Some(result) = result {
                        self.prepare_normal_shape(result, expected, prepared, depth + 1)?;
                    } else if !shape.diverges {
                        self.check_type_contract(
                            &nir::program::TypePredicate::Equal(
                                Term::Primitive(Primitive::Void),
                                expected.clone(),
                            ),
                            source,
                        )?;
                    }
                    return Ok(());
                }
                ShapeKind::Scalar(Input::Value(ty)) => {
                    self.prepare_normal_value_type(ty, expected, source)?;
                    return Ok(());
                }
                ShapeKind::Scalar(Input::Unit) => {
                    Prepared::Literal(self.normal_unit_value(expected, source)?)
                }
                ShapeKind::Scalar(Input::Integer { spelling, negative }) => Prepared::Literal(
                    self.normal_integer_value(expected, spelling, *negative, source)?,
                ),
                ShapeKind::List(items) => {
                    let element = self.normal_list_domain(expected, items.len(), source)?;
                    for (spread, child) in items {
                        self.prepare_normal_shape(
                            child,
                            if *spread { expected } else { &element },
                            prepared,
                            depth + 1,
                        )?;
                    }
                    Prepared::List {
                        ty: expected.clone(),
                        values: vec![],
                    }
                }
                ShapeKind::Tuple { head, items } => {
                    let type_tuple = *expected == Term::Primitive(Primitive::Type);
                    if type_tuple {
                        for child in items {
                            self.prepare_normal_shape(child, expected, prepared, depth + 1)?;
                        }
                        if let Some(head) = head {
                            if !self.normal_forms.get(head).is_some_and(|metadata| {
                                metadata.role == nir::normal_forms::Role::Tuple
                            }) {
                                return Err(self.failure(ErrorKind::Type, Some(source)));
                            }
                        }
                    } else {
                        let types = self
                            .tuple_input_elements(expected)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                        if types.len() != items.len() {
                            return Err(self.failure(ErrorKind::Arity, Some(source)));
                        }
                        for (child, ty) in items.iter().zip(types) {
                            self.prepare_normal_shape(child, ty, prepared, depth + 1)?;
                        }
                    }
                    Prepared::Tuple {
                        head: type_tuple.then_some(*head),
                        values: vec![],
                    }
                }
            };
        prepared.insert(shape.slot, value);
        Ok(())
    }
    pub(super) fn append_compiled_normal_container(
        &mut self,
        container: &mut Prepared,
        value: Value,
        spread: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        match container {
            Prepared::List { ty, values } => {
                self.append_normal_list_item(ty, values, spread, value, source)
            }
            Prepared::Tuple { values, .. } => {
                values.push(value);
                Ok(())
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub(super) fn finish_compiled_normal_container(
        &mut self,
        container: Prepared,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match container {
            Prepared::List { ty, values } => self.finish_normal_list(ty, values),
            Prepared::Tuple { head, values } => self.finish_contextual_tuple(head, values, source),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    /// All operands have already run. Preserve abstract Type authority and
    /// the registered tuple head regardless of when the input domain arrived.
    pub(super) fn finish_contextual_tuple(
        &mut self,
        type_head: Option<Option<DeclarationId>>,
        values: Vec<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match type_head {
            Some(head) => {
                let types = values
                    .into_iter()
                    .map(|value| self.type_description_term(value, source))
                    .collect::<Result<_, _>>()?;
                self.contextual_tuple_type(head, types, source)
            }
            None => Ok(Value::Tuple(values)),
        }
    }
    pub(super) fn normal_unit_value(
        &mut self,
        expected: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let value = match expected {
            Term::Primitive(Primitive::Type) => Value::Type(Term::Primitive(Primitive::Void)),
            _ => {
                self.check_type_contract(
                    &nir::program::TypePredicate::Equal(
                        Term::Primitive(Primitive::Void),
                        expected.clone(),
                    ),
                    source,
                )?;
                Value::Unit
            }
        };
        Ok(value)
    }
    pub(super) fn normal_integer_value(
        &mut self,
        expected: &Term,
        spelling: &str,
        negative: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let integer = self.integer_literal(spelling, Some(source))?;
        let integer = if negative {
            self.integer_operation(Some(source), |budget| integer.negate(budget))?
        } else {
            integer
        };
        let value = match expected {
            Term::Primitive(Primitive::Integer) => Value::Integer(integer),
            Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                self.bit_operation(source, |budget| integer.to_bitvec(*width, budget))?,
            ),
            Term::Primitive(primitive) if self.integer_layout(*primitive).is_some() => {
                let (signed, bits) = self.integer_layout(*primitive).unwrap();
                if signed {
                    Value::SignedInteger(
                        integer
                            .to_signed(bits)
                            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                    )
                } else {
                    Value::UnsignedInteger(
                        integer
                            .to_unsigned(bits)
                            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                    )
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        Ok(value)
    }
    pub(super) fn prepare_normal_value_type(
        &mut self,
        ty: &Term,
        expected: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if ty != expected && *ty != Term::Primitive(Primitive::Never) {
            self.check_type_contract(
                &nir::program::TypePredicate::Equal(ty.clone(), expected.clone()),
                source,
            )?;
        }
        Ok(())
    }
    pub(super) fn bind_compiled_normal_input(
        &mut self,
        inputs: &mut Inputs,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let normal = inputs
            .normal
            .as_mut()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let slot = normal.bindings.arguments.len();
        self.bind_normal_input(normal.head, &mut normal.bindings, slot, value, source)
    }
    pub(super) fn bind_normal_input(
        &mut self,
        head: DeclarationId,
        bindings: &mut Bindings,
        slot: usize,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        if slot >= bindings.slots.len() || bindings.contains(slot) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let parameter = self
            .normal_forms
            .get(&head)
            .and_then(|head| head.parameters.get(slot))
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let charged = nir::type_computations::charge_term(&parameter.ty, &mut remaining);
        self.budget.steps -= (before - remaining) as u64;
        charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        let domain = parameter.ty.clone();
        let parameter = nir::DeclarationParameter {
            declaration: parameter.declaration,
            sort: if parameter.kind == nir::normal_forms::Kind::Type {
                nir::Sort::Type
            } else {
                nir::Sort::ComptimeValue
            },
        };
        let term = self.residual_input_term(value, source)?;
        bindings.slots[slot] = Some(bindings.arguments.len());
        bindings.parameters.push(parameter);
        bindings.domains.push(domain);
        bindings.arguments.push(term);
        Ok(())
    }
}

/// Prepared domains and accumulators are invocation-local, never code-cache data.
pub(super) enum Prepared {
    Literal(Value),
    List {
        ty: Term,
        values: Vec<Value>,
    },
    Tuple {
        head: Option<Option<DeclarationId>>,
        values: Vec<Value>,
    },
}
impl fmt::Display for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, values) = match self {
            Self::Literal(value) => return write!(f, "(literal {value})"),
            Self::List { ty, values } => {
                write!(f, "(list (type {ty})")?;
                ("values", values)
            }
            Self::Tuple { head, values } => {
                write!(f, "(tuple (type-tuple {}) (head", head.is_some())?;
                if let Some(Some(head)) = head {
                    write!(f, " {head}")?;
                }
                write!(f, ")")?;
                ("values", values)
            }
        };
        write!(f, " ({kind}")?;
        for value in values {
            write!(f, " {value}")?;
        }
        write!(f, "))")
    }
}
