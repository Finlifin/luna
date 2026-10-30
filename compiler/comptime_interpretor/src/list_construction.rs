//! List construction checks ordinary element domains and preserves list identity.
use super::*;
impl Interpreter<'_> {
    pub(super) fn prepare_list_construction<'b>(
        &mut self,
        ty: &Term,
        fields: impl ExactSizeIterator<Item = &'b Term> + Clone,
        source: SourceRange,
    ) -> Result<Option<Term>, Error> {
        if fields.len() > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        let diverging = fields
            .clone()
            .any(|field| *field == Term::Primitive(Primitive::Never));
        let element_type = self.list_element(ty).cloned().or_else(|| {
            self.is_type_list_input(ty)
                .then_some(Term::Primitive(Primitive::Type))
        });
        if (diverging && *ty != Term::Primitive(Primitive::Never))
            || (!diverging && element_type.is_none())
            || fields.clone().any(|field| {
                *field != Term::Primitive(Primitive::Never)
                    && !diverging
                    && Some(field) != element_type.as_ref()
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.integer_operation(Some(source), |budget| {
            budget.consume(fields.len().saturating_add(1))
        })?;
        Ok(element_type)
    }

    pub(super) fn complete_list_construction(
        &mut self,
        ty: &Term,
        values: Vec<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let element = self
            .list_element(ty)
            .cloned()
            .or_else(|| {
                self.is_type_list_input(ty)
                    .then_some(Term::Primitive(Primitive::Type))
            })
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let value = if element == Term::Primitive(Primitive::Type) {
            Value::TypeList(
                values
                    .into_iter()
                    .map(|value| self.type_description_term(value, source))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            Value::List {
                ty: ty.clone(),
                values,
            }
        };
        self.validate_immutable_input(&value, ty)?;
        Ok(value)
    }

    /// Decode a checked list of Type values at a host definition boundary.
    /// The legacy compact carrier is accepted during VM representation migration.
    pub(super) fn definition_type_elements(
        &mut self,
        value: Value,
        source: SourceRange,
    ) -> Result<Vec<Term>, Error> {
        match value {
            Value::TypeList(elements) => Ok(elements),
            Value::List { ty, values } if self.is_type_list_input(&ty) => {
                self.integer_operation(Some(source), |budget| budget.consume(values.len()))?;
                values
                    .into_iter()
                    .map(|value| self.type_description_term(value, source))
                    .collect()
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    pub(super) fn eval_type_list(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeList(fields) = &expr.kind else {
            unreachable!()
        };
        let element = self.prepare_list_construction(
            &expr.ty,
            fields.iter().map(|field| &field.ty),
            expr.source,
        )?;
        let diverging = fields
            .iter()
            .any(|field| field.ty == Term::Primitive(Primitive::Never));
        if diverging {
            for field in fields {
                let _ = value!(self.eval(field, environment));
            }
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let mut values = Vec::with_capacity(fields.len());
        for field in fields {
            let value = value!(self.eval(field, environment));
            values.push(
                if !diverging && element == Some(Term::Primitive(Primitive::Type)) {
                    Value::Type(self.type_description_term(value, field.source)?)
                } else {
                    value
                },
            );
        }
        self.complete_list_construction(&expr.ty, values, expr.source)
            .map(Flow::Value)
    }
}
