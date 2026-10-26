//! Immutable list operations retain identity, evaluation order and VM limits.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_list_operation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (left, right) = match &expr.kind {
            K::ListLength(left) => (left, None),
            K::ListConcat(left, right) => (left, Some(right)),
            _ => unreachable!(),
        };
        let never = Term::Primitive(Primitive::Never);
        let diverges = left.ty == never || right.is_some_and(|right| right.ty == never);
        let expected = if diverges {
            &never
        } else if right.is_some() {
            &left.ty
        } else {
            &Term::Primitive(Primitive::Integer)
        };
        if expr.ty != *expected
            || (left.ty != never
                && self.list_element(&left.ty).is_none()
                && !self.is_type_list_input(&left.ty))
            || right
                .is_some_and(|right| right.ty != never && left.ty != never && right.ty != left.ty)
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let mut value = match self.eval(left, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.validate_immutable_input(&value, &left.ty)?;
        let length = match &value {
            Value::TypeList(values) => values.len(),
            Value::List { values, .. } => values.len(),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let Some(right) = right else {
            let count = self.integer_operation(Some(expr.source), |budget| {
                budget.consume(1)?;
                Ok(comptime_runtime::Integer::from_u128(length as u128))
            })?;
            return Ok(Flow::Value(Value::Integer(count)));
        };
        let other = match self.eval(right, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.validate_immutable_input(&other, &right.ty)?;
        let other_length = match &other {
            Value::TypeList(values) => values.len(),
            Value::List { values, .. } => values.len(),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let count = length
            .checked_add(other_length)
            .filter(|count| *count <= 100_000)
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(count.saturating_add(1))
        })?;
        match (&mut value, other) {
            (Value::TypeList(left), Value::TypeList(right)) => left.extend(right),
            (Value::List { values: left, .. }, Value::List { values: right, .. }) => {
                left.extend(right)
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        }
        // Both inputs were admitted. A combined immutable description can
        // still exceed the shared structural work limit even below the element
        // limit; report that as a resource limit rather than a malformed type.
        let work = value
            .scoped_copy_work_with_evidence(
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
            )
            .ok_or_else(|| self.failure(ErrorKind::RepresentationSizeLimit, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
        Ok(Flow::Value(value))
    }
}
