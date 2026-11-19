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
        if let Value::Description(ref data) = operand {
            if !self.admits_description(data) {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            let Some(world) = self.declaration_world else {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(expr.source)));
            };
            let subject_term = Term::Rigid {
                declaration: DeclarationId {
                    package: u64::MAX,
                    local: 0,
                },
                sort: nir::Sort::Type,
            };
            let before = self.budget.steps;
            let subject = world
                .refinements
                .subject(data, &mut self.budget.steps)
                .map_err(|_| {
                    self.failure(
                        if self.budget.steps == 0 && before > 0 {
                            ErrorKind::StepsExhausted
                        } else {
                            ErrorKind::Type
                        },
                        Some(expr.source),
                    )
                })?;
            let mut premises = Vec::new();
            if let Some(app) = subject.application
                && self
                    .normal_forms
                    .get(&app.head())
                    .is_some_and(|m| m.role == nir::normal_forms::Role::Tuple)
            {
                premises.push(nir::program::TypePredicate::Kind(
                    subject_term.clone(),
                    nir::TypeKind::Tuple,
                ));
            }
            for req in subject.requirements() {
                let instantiated = self.query_admission(Some(expr.source), |_, fuel| {
                    Ok(req.instantiate(&subject_term, &[], fuel)?)
                })?;
                premises.push(instantiated);
            }
            let target = nir::program::TypePredicate::Kind(subject_term, *kind);
            let implies = |consequent: nir::program::TypePredicate| {
                if premises.is_empty() {
                    consequent
                } else {
                    nir::program::TypePredicate::Or(vec![
                        nir::program::TypePredicate::Not(Box::new(
                            nir::program::TypePredicate::And(premises.clone()),
                        )),
                        consequent,
                    ])
                }
            };
            let pos = implies(target.clone());
            self.tick(Some(expr.source))?;
            if pos.trivial_truth() == Some(true)
                || premises.iter().any(|p| p == &target)
                || self
                    .solve_type_predicate(&pos)
                    .map_err(|k| self.failure(k, Some(expr.source)))?
                    == ContractOutcome::Satisfied
            {
                return Ok(Flow::Value(Value::Bool(true)));
            }
            let neg = implies(nir::program::TypePredicate::Not(Box::new(target)));
            self.tick(Some(expr.source))?;
            if neg.trivial_truth() == Some(true)
                || self
                    .solve_type_predicate(&neg)
                    .map_err(|k| self.failure(k, Some(expr.source)))?
                    == ContractOutcome::Satisfied
            {
                return Ok(Flow::Value(Value::Bool(false)));
            }
            return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(expr.source)));
        }
        let operand = self.type_identity_term(operand, expr.source)?;
        self.complete_type_kind(*kind, operand, expr.source)
            .map(Flow::Value)
    }
}
