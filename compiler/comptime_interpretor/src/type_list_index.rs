//! Leaf indexing of immutable comptime type lists without copying unrelated elements.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_local_type_list_index(
        &mut self,
        expr: &Expr,
        values: &Expr,
        binding: LocalId,
        index: &Expr,
        environment: &Environment,
    ) -> Result<Flow, Error> {
        let result_type = if index.ty == Term::Primitive(Primitive::Never) {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if expr.ty != Term::Primitive(result_type)
            || !matches!(
                index.ty,
                Term::Primitive(Primitive::Integer | Primitive::Never)
            )
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        // Preserve the local-read tick and missing-binding/lock diagnostics.
        // The caller excludes cleanup schedules and admits only a literal or local
        // index, so no code can mutate the borrowed list during this read.
        self.tick(Some(values.source))?;
        let local = environment
            .get(&binding)
            .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(values.source)))?;
        let storage = if let Value::Storage(storage) = local {
            Some(
                storage
                    .0
                    .try_lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(values.source)))?,
            )
        } else {
            None
        };
        let referenced = if let Value::ComptimeReference(reference) = local {
            Some(
                reference
                    .borrow(&mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(expr.source)))?,
            )
        } else {
            None
        };
        let list = referenced
            .as_ref()
            .map(|value| value.value())
            .unwrap_or_else(|| storage.as_deref().unwrap_or(local));
        let Value::TypeList(elements) = list else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let length = elements.len();
        let evaluated = match index.kind {
            K::Local(binding) => {
                // A local read cannot mutate the borrowed list. Keep the usual
                // read tick, representation budget and missing-binding errors.
                self.tick(Some(index.source))?;
                self.local_value(environment, binding, Some(index.source))?
            }
            _ => match self.eval(index, &mut Environment::new())? {
                Flow::Value(value) => value,
                flow => return Ok(flow),
            },
        };
        let Value::Integer(position) = evaluated else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let position = position
            .to_u128()
            .ok()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < length)
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;
        let result = &elements[position];
        let work = self
            .type_description_work(result)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
        Ok(Flow::Value(Value::Type(result.clone())))
    }
}
