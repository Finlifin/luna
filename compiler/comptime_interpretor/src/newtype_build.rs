//! Concrete newtype requests for checked host publication.
use super::*;

#[derive(Debug)]
pub struct NewtypeBuildRequest {
    pub name: String,
    pub representation: Term,
    pub source: SourceRange,
}
impl NewtypeBuildRequest {
    /// Reserve descriptor assembly before copying its name or representation.
    pub fn charge_assembly(&self, fuel: &mut usize) -> Result<(), ()> {
        *fuel = fuel
            .checked_sub(self.name.len().saturating_add(1))
            .ok_or(())?;
        nir::type_computations::charge_term(&self.representation, fuel).map_err(|_| ())
    }
}
impl fmt::Display for NewtypeBuildRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(newtype-build (name {}) (representation {}))",
            nir::QuotedDisplay(&self.name),
            self.representation
        )
    }
}

/// The publisher owns identity allocation and ordinary declaration checking.
/// Commit must be transactional; dropping an uncommitted candidate publishes
/// nothing. VM admission separately checks the exact retained descriptors.
pub struct PreparedNewtypeBuild<'a> {
    pub definitions: Arc<[nir::newtypes::Newtype]>,
    pub result: DeclarationId,
    pub commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
}

impl<'a> Interpreter<'a> {
    pub fn with_newtype_builder(
        mut self,
        builder: &'a dyn Fn(
            &NewtypeBuildRequest,
            &mut u64,
        ) -> Result<PreparedNewtypeBuild<'a>, Error>,
    ) -> Self {
        self.newtype_builder = Some(builder);
        self
    }

    pub(super) fn build_newtype_value(
        &mut self,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Value::Tuple(values) = value else {
            return Err(fail());
        };
        let mut values = values.into_iter();
        let (Some(Value::Symbol(name)), Some(Value::Type(representation)), None) =
            (values.next(), values.next(), values.next())
        else {
            return Err(fail());
        };
        self.integer_operation(Some(source), |budget| {
            budget.consume(name.len().saturating_add(1))
        })?;
        self.admit_type(&representation, 0)?;
        let request = NewtypeBuildRequest {
            name: name.to_string(),
            representation,
            source,
        };
        let builder = self.newtype_builder.ok_or_else(fail)?;
        let prepared = host_budget::clamped(&mut self.budget.steps, |remaining| {
            builder(&request, remaining)
        })?;
        self.integer_operation(Some(source), |budget| {
            budget.consume(prepared.definitions.len())
        })?;
        let definition = prepared
            .definitions
            .iter()
            .find(|item| item.declaration == prepared.result)
            .ok_or_else(fail)?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let checked = (|| {
            fuel = fuel
                .checked_sub(definition.name.len().saturating_add(1))
                .ok_or(())?;
            nir::type_computations::charge_term(&definition.representation, &mut fuel)
                .map_err(|_| ())
        })();
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        if definition.name != request.name
            || definition.representation != request.representation
            || !definition.type_parameters.is_empty()
            || !definition.parameter_types.is_empty()
            || definition.capture_count != 0
            || !definition.requirements.is_empty()
            || !definition.capabilities.is_empty()
        {
            return Err(fail());
        }
        let result = Term::Nominal {
            declaration: prepared.result,
            arguments: vec![],
        };
        self.install_owned_generated_newtypes_with(prepared.definitions, prepared.commit)
            .map_err(|error| match error {
                NewtypePublicationError::Admission(error)
                | NewtypePublicationError::Commit(error) => error,
            })?;
        Ok(Value::Type(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn expression() -> Expr {
        let source = SourceRange { start: 1, end: 2 };
        Expr {
            source,
            ty: Term::Primitive(Primitive::Type),
            kind: K::BuildNewtype(Box::new(Expr {
                source,
                ty: Term::Tuple(vec![
                    Term::Primitive(Primitive::Symbol),
                    Term::Primitive(Primitive::Type),
                ]),
                kind: K::Tuple(vec![
                    Expr {
                        source,
                        ty: Term::Primitive(Primitive::Symbol),
                        kind: K::Literal(Literal::Symbol("Flag".into())),
                    },
                    Expr {
                        source,
                        ty: Term::Primitive(Primitive::Type),
                        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
                    },
                ]),
            })),
        }
    }
    #[test]
    fn request_rejects_host_replacement_before_commit_in_both_carriers() {
        let package = Package::default();
        let body = expression();
        let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
        assert!(body.to_string().contains("(build-newtype"));
        assert!(code.to_string().contains("(definition-build newtype"));
        for compiled in [false, true] {
            for alteration in 0..4 {
                let committed = Cell::new(false);
                let builder = |request: &NewtypeBuildRequest, _: &mut u64| {
                    let mut definition = nir::newtypes::Newtype {
                        declaration: DeclarationId {
                            package: 93,
                            local: 1,
                        },
                        name: request.name.clone(),
                        type_parameters: vec![],
                        parameter_types: vec![],
                        capture_count: 0,
                        requirements: vec![],
                        capabilities: vec![],
                        representation: request.representation.clone(),
                    };
                    match alteration {
                        1 => definition.representation = Term::Primitive(Primitive::Char),
                        2 => definition.name = "Replaced".into(),
                        3 => definition
                            .requirements
                            .push(nir::program::TypePredicate::Bool(true)),
                        _ => {}
                    }
                    Ok(PreparedNewtypeBuild {
                        result: definition.declaration,
                        definitions: vec![definition].into(),
                        commit: Box::new(|| {
                            committed.set(true);
                            Ok(())
                        }),
                    })
                };
                let mut vm = Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .with_newtype_builder(&builder);
                let result = if compiled {
                    vm.execute_code(&code, &mut Environment::new())
                } else {
                    vm.eval(&body, &mut Environment::new())
                };
                if alteration == 0 {
                    assert!(matches!(
                        result,
                        Ok(Flow::Value(Value::Type(Term::Nominal { .. })))
                    ));
                    assert!(committed.get());
                } else {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                    assert!(!committed.get());
                }
            }
        }
    }
    #[test]
    fn runtime_request_never_calls_the_publisher() {
        let package = Package::default();
        let called = Cell::new(false);
        let builder = |_: &NewtypeBuildRequest, _: &mut u64| {
            called.set(true);
            Err(Error {
                kind: ErrorKind::Type,
                source: None,
            })
        };
        let mut vm = Interpreter::new(&package, Budget::default())
            .unwrap()
            .with_newtype_builder(&builder);
        assert_eq!(
            vm.eval(&expression(), &mut Environment::new())
                .unwrap_err()
                .kind,
            ErrorKind::PhaseMismatch
        );
        assert!(!called.get());
    }
}
