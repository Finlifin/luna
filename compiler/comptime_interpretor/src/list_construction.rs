//! List construction checks ordinary element domains and preserves list identity.
use super::*;
impl Interpreter<'_> {
    pub(super) fn prepare_list_construction<'b>(
        &mut self,
        ty: &Term,
        fields: impl ExactSizeIterator<Item = &'b Term> + Clone,
        source: SourceRange,
    ) -> Result<(Term, Option<Term>), Error> {
        if fields.len() > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        self.integer_operation(Some(source), |budget| {
            budget.consume(fields.len().saturating_add(1))
        })?;
        let ty = self.selected_type_view(ty, Some(source))?;
        let fields = fields
            .map(|field| self.selected_type_view(field, Some(source)))
            .collect::<Result<Vec<_>, _>>()?;
        let diverging = fields
            .iter()
            .any(|field| *field == Term::Primitive(Primitive::Never));
        let element_type = self.list_element(&ty).cloned();
        if (diverging && ty != Term::Primitive(Primitive::Never))
            || (!diverging && element_type.is_none())
            || fields.iter().any(|field| {
                *field != Term::Primitive(Primitive::Never)
                    && !diverging
                    && Some(field) != element_type.as_ref()
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok((ty, element_type))
    }

    pub(super) fn complete_list_construction(
        &mut self,
        ty: &Term,
        values: Vec<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if self.list_element(ty).is_none() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let value = Value::List {
            ty: ty.clone(),
            values: values.into(),
        };
        self.validate_immutable_input(&value, ty)?;
        Ok(value)
    }

    /// Decode a checked list of Type values at a host definition boundary.
    pub(super) fn definition_type_elements(
        &mut self,
        value: Value,
        source: SourceRange,
    ) -> Result<Vec<Term>, Error> {
        match value {
            Value::List { ty, values }
                if self.list_element(&ty) == Some(&Term::Primitive(Primitive::Type)) =>
            {
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
        let (ty, element) = self.prepare_list_construction(
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
        self.complete_list_construction(&ty, values, expr.source)
            .map(Flow::Value)
    }
}
