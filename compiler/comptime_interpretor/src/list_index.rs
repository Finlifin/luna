//! Immutable indexing shares domain checks and avoids copying unrelated elements.
use super::*;

impl Interpreter<'_> {
    pub(super) fn prepare_list_index(
        &self,
        ty: &Term,
        list: &Term,
        index: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let element = self
            .list_element(list)
            .cloned()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let expected = if *index == Term::Primitive(Primitive::Never) {
            index
        } else {
            &element
        };
        if ty != expected
            || !matches!(
                index,
                Term::Primitive(Primitive::Integer | Primitive::Never)
            )
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }
    pub(super) fn index_list_length(
        &mut self,
        value: &Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<usize, Error> {
        let work = self
            .type_description_work(ty)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(work))?;
        match value {
            Value::List { ty: stored, values }
                if stored == ty && self.list_element(ty).is_some() =>
            {
                Ok(values.len())
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    fn list_position(
        &self,
        value: Value,
        length: usize,
        source: SourceRange,
    ) -> Result<usize, Error> {
        let Value::Integer(position) = value else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        position
            .to_u128()
            .ok()
            .and_then(|p| usize::try_from(p).ok())
            .filter(|p| *p < length)
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))
    }
    pub(super) fn complete_list_index(
        &mut self,
        list: Value,
        index: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let length = self.list_value_length(&list, source)?;
        let position = self.list_position(index, length, source)?;
        match list {
            Value::List { mut values, .. } => Ok(values.swap_remove(position)),
            _ => unreachable!(),
        }
    }
    pub(super) fn read_local_list_index(
        &mut self,
        local: &Value,
        ty: &Term,
        independent: bool,
        value_source: SourceRange,
        source: SourceRange,
        index: impl FnOnce(&mut Self) -> Result<Flow, Error>,
    ) -> Result<Flow, Error> {
        if !independent {
            let snapshot = self.invocation_slot_value(local, Some(value_source))?;
            self.index_list_length(&snapshot, ty, source)?;
            let value = match index(self)? {
                Flow::Value(value) => value,
                flow => return Ok(flow),
            };
            return self
                .complete_list_index(snapshot, value, source)
                .map(Flow::Value);
        }
        let storage = if let Value::Storage(storage) = local {
            Some(
                storage
                    .0
                    .try_lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(value_source)))?,
            )
        } else {
            None
        };
        let referenced = if let Value::ComptimeReference(reference) = local {
            Some(
                reference
                    .borrow(&mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(source)))?,
            )
        } else {
            None
        };
        let list = referenced
            .as_ref()
            .map(|v| v.value())
            .unwrap_or_else(|| storage.as_deref().unwrap_or(local));
        let length = self.index_list_length(list, ty, source)?;
        let value = match index(self)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let position = self.list_position(value, length, source)?;
        let value = match list {
            Value::List { values, .. } => {
                self.checkpoint_representation(&values[position], source)?
            }
            _ => unreachable!(),
        };
        Ok(Flow::Value(value))
    }
    pub(super) fn eval_type_list_index(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeListIndex(values, index) = &expr.kind else {
            unreachable!()
        };
        self.prepare_list_index(&expr.ty, &values.ty, &index.ty, expr.source)?;
        if self.comptime_only
            && self.cleanup_schedule.is_none()
            && let K::Local(binding) = values.kind
            && matches!(index.kind, K::Local(_) | K::Literal(Literal::Integer(_)))
        {
            self.tick(Some(values.source))?;
            let local = environment
                .get(&binding)
                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(values.source)))?;
            let independent = match index.kind {
                K::Local(binding) => matches!(environment.get(&binding), Some(Value::Integer(_))),
                _ => true,
            };
            return self.read_local_list_index(
                local,
                &values.ty,
                independent,
                values.source,
                expr.source,
                |this| match index.kind {
                    K::Local(binding) => {
                        this.tick(Some(index.source))?;
                        this.local_value(environment, binding, Some(index.source))
                            .map(Flow::Value)
                    }
                    _ => this.eval(index, &mut Environment::new()),
                },
            );
        }
        let list = match self.eval(values, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.index_list_length(&list, &values.ty, expr.source)?;
        let index = match self.eval(index, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        self.complete_list_index(list, index, expr.source)
            .map(Flow::Value)
    }
}
