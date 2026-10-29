//! Type equality asks the host theory without unfolding normal-form heads.
use super::*;

impl Interpreter<'_> {
    pub(super) fn prepare_type_equal(
        &self,
        ty: &Term,
        left: &Term,
        right: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let expected = if [left, right]
            .iter()
            .any(|field| **field == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Bool
        };
        if *ty != Term::Primitive(expected)
            || [left, right].iter().any(|field| {
                !matches!(**field, Term::Primitive(Primitive::Type | Primitive::Never))
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn complete_type_equal(
        &mut self,
        left: Term,
        right: Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let work = self
            .type_description_work(&left)
            .and_then(|a| {
                self.type_description_work(&right)
                    .and_then(|b| a.checked_add(b))
            })
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(work))?;
        let predicate = nir::program::TypePredicate::Equal(left, right);
        if predicate.trivial_truth() == Some(true) {
            return Ok(Value::Bool(true));
        }
        // Failure to entail equality is not evidence of inequality. Ask for
        // a proof of its negation separately; Unknown must not select a branch.
        if self.type_solver.is_some() {
            self.tick(Some(source))?;
            if self.solve_type_predicate(&predicate) == ContractOutcome::Satisfied {
                return Ok(Value::Bool(true));
            }
            self.tick(Some(source))?;
            if self.solve_type_predicate(&nir::program::TypePredicate::Not(Box::new(predicate)))
                == ContractOutcome::Satisfied
            {
                return Ok(Value::Bool(false));
            }
        }
        Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)))
    }

    pub(super) fn eval_type_equal(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeEqual(left, right) = &expr.kind else {
            unreachable!()
        };
        self.prepare_type_equal(&expr.ty, &left.ty, &right.ty, expr.source)?;
        let left = value!(self.eval(left, environment));
        let left = self.type_description_term(left, expr.source)?;
        let right = value!(self.eval(right, environment));
        let right = self.type_description_term(right, expr.source)?;
        self.complete_type_equal(left, right, expr.source)
            .map(Flow::Value)
    }
}
