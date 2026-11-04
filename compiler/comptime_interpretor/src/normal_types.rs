//! Invocation-local bindings and contextual normal-form construction.
use super::*;

use input_bindings::Bindings;

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
        self.prepare_input_domain(
            input_domains::Node::Compiled(shape),
            expected,
            InputKey::Register(shape.slot),
            depth,
            &mut |key, value| {
                let InputKey::Register(slot) = key else {
                    unreachable!("compiled input identity")
                };
                prepared.insert(slot, value);
            },
        )
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
            Prepared::Tuple { values } => Ok(Value::Tuple(values)),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub(super) fn normal_unit_value(
        &mut self,
        expected: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        // Match ordinary expression elaboration: unit syntax may construct an
        // empty tuple value when the contextual domain supplies that layout.
        if self
            .tuple_input_elements(expected)
            .is_some_and(|fields| fields.is_empty())
        {
            return Ok(Value::Tuple(Vec::new()));
        }
        self.check_type_contract(
            &nir::program::TypePredicate::Equal(Term::Primitive(Primitive::Void), expected.clone()),
            source,
        )?;
        Ok(Value::Unit)
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
        let parameter = self
            .normal_forms
            .get(&head)
            .and_then(|head| head.parameters.get(slot))
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let domain = &parameter.ty;
        let parameter = nir::DeclarationParameter {
            declaration: parameter.declaration,
            sort: if parameter.kind == nir::normal_forms::Kind::Type {
                nir::Sort::Type
            } else {
                nir::Sort::ComptimeValue
            },
        };
        bindings.bind(self, slot, parameter, domain, value, source)
    }
}

/// Prepared domains and accumulators are invocation-local, never code-cache data.
pub(super) enum Prepared {
    Literal(Value),
    List { ty: Term, values: Vec<Value> },
    Tuple { values: Vec<Value> },
}
impl fmt::Display for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, values) = match self {
            Self::Literal(value) => return write!(f, "(literal {value})"),
            Self::List { ty, values } => {
                write!(f, "(list (type {ty})")?;
                ("values", values)
            }
            Self::Tuple { values } => {
                write!(f, "(tuple")?;
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
