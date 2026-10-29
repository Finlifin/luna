//! Checked record and variant assembly, shared across execution carriers.
//! Tree evaluation additionally retains scheduled prepared-value cleanup.
use super::*;

impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn eval_variant(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let K::Variant {
                declaration,
                index,
                fields,
            } = &expr.kind
            else {
                unreachable!("variant evaluator")
            };
            let mut values = Vec::new();
            for (slot, field) in fields.iter().enumerate() {
                let value = match this.eval(field, environment)? {
                    Flow::Value(value) => value,
                    control => return Ok(control),
                };
                this.retain_prepared(slot, field.source, &value)?;
                values.push(value);
            }
            this.construct_variant(&expr.ty, *declaration, *index, values, expr.source)
                .map(Flow::Value)
        })
    }

    pub(super) fn construct_variant(
        &mut self,
        ty: &Term,
        declaration: DeclarationId,
        index: usize,
        values: Vec<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let selected = if self.associated_values.is_empty() {
            None
        } else {
            Some(self.selected_type_view(ty, Some(source))?)
        };
        let Term::Nominal { arguments, .. } = selected.as_ref().unwrap_or(ty) else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let value = Value::Variant {
            arguments: arguments.clone(),
            declaration,
            index,
            fields: values,
        };
        if !self.has_type(&value, ty) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }

    #[inline(never)]
    pub(super) fn eval_record(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let (declaration, fields) = match &expr.kind {
                K::AnonymousRecord { fields } => (None, fields),
                K::Record {
                    declaration,
                    fields,
                } => (Some(*declaration), fields),
                _ => unreachable!("record evaluator"),
            };
            let mut values = Vec::new();
            for (slot, field) in fields.iter().enumerate() {
                let value = match this.eval(field, environment)? {
                    Flow::Value(value) => value,
                    control => return Ok(control),
                };
                this.retain_prepared(slot, field.source, &value)?;
                values.push(value);
            }
            this.construct_record(&expr.ty, declaration, values, expr.source)
                .map(Flow::Value)
        })
    }

    pub(super) fn construct_record(
        &mut self,
        ty: &Term,
        declaration: Option<DeclarationId>,
        values: Vec<Value>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let value = if let Some(declaration) = declaration {
            let selected = if self.associated_values.is_empty() {
                None
            } else {
                Some(self.selected_type_view(ty, Some(source))?)
            };
            let Term::Nominal { arguments, .. } = selected.as_ref().unwrap_or(ty) else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            Value::Record {
                arguments: arguments.clone(),
                declaration,
                fields: values,
            }
        } else {
            let Term::Record(types) = ty else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            if types.len() != values.len() {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            Value::AnonymousRecord(
                types
                    .iter()
                    .map(|(name, _)| name.clone())
                    .zip(values)
                    .collect(),
            )
        };
        if !self.has_type(&value, ty) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }
}
