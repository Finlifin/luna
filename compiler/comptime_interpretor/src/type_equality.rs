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
        self.check_type_description(&left, Some(source))?;
        self.check_type_description(&right, Some(source))?;
        let predicate = nir::program::TypePredicate::Equal(left, right);
        if predicate.trivial_truth() == Some(true) {
            return Ok(Value::Bool(true));
        }
        // Failure to entail equality is not evidence of inequality. Ask for
        // a proof of its negation separately; Unknown must not select a branch.
        if self.type_solver.is_some() {
            self.tick(Some(source))?;
            if self
                .solve_type_predicate(&predicate)
                .map_err(|kind| self.failure(kind, Some(source)))?
                == ContractOutcome::Satisfied
            {
                return Ok(Value::Bool(true));
            }
            self.tick(Some(source))?;
            if self
                .solve_type_predicate(&nir::program::TypePredicate::Not(Box::new(predicate)))
                .map_err(|kind| self.failure(kind, Some(source)))?
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
        let left = self.type_identity_term(left, expr.source)?;
        let right = value!(self.eval(right, environment));
        let right = self.type_identity_term(right, expr.source)?;
        self.complete_type_equal(left, right, expr.source)
            .map(Flow::Value)
    }

    pub(super) fn prepare_type_kind(
        &self,
        ty: &Term,
        operand: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        let expected = if *operand == Term::Primitive(Primitive::Never) {
            Primitive::Never
        } else {
            Primitive::Bool
        };
        if *ty != Term::Primitive(expected)
            || !matches!(operand, Term::Primitive(Primitive::Type | Primitive::Never))
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn complete_type_kind(
        &mut self,
        kind: nir::TypeKind,
        operand: Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        self.check_type_description(&operand, Some(source))?;
        let predicate = nir::program::TypePredicate::Kind(operand, kind);
        self.tick(Some(source))?;
        if self
            .solve_type_predicate(&predicate)
            .map_err(|kind| self.failure(kind, Some(source)))?
            == ContractOutcome::Satisfied
        {
            return Ok(Value::Bool(true));
        }
        self.tick(Some(source))?;
        if self
            .solve_type_predicate(&nir::program::TypePredicate::Not(Box::new(predicate)))
            .map_err(|kind| self.failure(kind, Some(source)))?
            == ContractOutcome::Satisfied
        {
            return Ok(Value::Bool(false));
        }
        Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)))
    }

    pub(super) fn complete_type_kind_value(
        &mut self,
        kind: nir::TypeKind,
        operand: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if let Value::Description(ref data) = operand {
            use nir::program::TypePredicate as P;
            let mut evidence = self.description_evidence(data, None, source)?;
            let target = P::Kind(evidence.subject.clone(), kind);
            if evidence.entails(self, target.clone(), source)? {
                return Ok(Value::Bool(true));
            }
            if evidence.entails(self, P::Not(Box::new(target)), source)? {
                return Ok(Value::Bool(false));
            }
            return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
        }
        let operand = self.type_identity_term(operand, source)?;
        self.complete_type_kind(kind, operand, source)
    }

    pub(super) fn eval_type_kind(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeKind { kind, operand } = &expr.kind else {
            unreachable!()
        };
        self.prepare_type_kind(&expr.ty, &operand.ty, expr.source)?;
        let operand = value!(self.eval(operand, environment));
        self.complete_type_kind_value(*kind, operand, expr.source)
            .map(Flow::Value)
    }

    pub(super) fn complete_type_not_sized(
        &mut self,
        operand: Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        self.check_type_description(&operand, Some(source))?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let concrete = nir::layout::evaluate_not_sized(
            &observations::InterpreterLayoutWorld(self),
            &operand,
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        match concrete {
            Ok(Some(value)) => return Ok(Value::Bool(value)),
            Ok(None) => {}
            Err(nir::LayoutError::BudgetExhausted) => {
                return Err(self.failure(ErrorKind::StepsExhausted, Some(source)));
            }
            Err(nir::LayoutError::ComptimeOnly(_)) => {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            Err(_) => {}
        }
        let predicate = nir::program::TypePredicate::NotSized(operand);
        self.tick(Some(source))?;
        if self
            .solve_type_predicate(&predicate)
            .map_err(|kind| self.failure(kind, Some(source)))?
            == ContractOutcome::Satisfied
        {
            return Ok(Value::Bool(true));
        }
        self.tick(Some(source))?;
        if self
            .solve_type_predicate(&nir::program::TypePredicate::Not(Box::new(predicate)))
            .map_err(|kind| self.failure(kind, Some(source)))?
            == ContractOutcome::Satisfied
        {
            return Ok(Value::Bool(false));
        }
        Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)))
    }

    pub(super) fn complete_type_not_sized_value(
        &mut self,
        operand: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if let Value::Description(ref data) = operand {
            use nir::program::TypePredicate as P;
            let mut evidence = self.description_evidence(data, None, source)?;
            let target = P::NotSized(evidence.subject.clone());
            if evidence.entails(self, target.clone(), source)? {
                return Ok(Value::Bool(true));
            }
            if evidence.entails(self, P::Not(Box::new(target)), source)? {
                return Ok(Value::Bool(false));
            }
            return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
        }
        let operand = self.type_identity_term(operand, source)?;
        self.complete_type_not_sized(operand, source)
    }

    pub(super) fn eval_type_not_sized(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeNotSized { operand } = &expr.kind else {
            unreachable!()
        };
        self.prepare_type_kind(&expr.ty, &operand.ty, expr.source)?;
        let operand = value!(self.eval(operand, environment));
        self.complete_type_not_sized_value(operand, expr.source)
            .map(Flow::Value)
    }
}
