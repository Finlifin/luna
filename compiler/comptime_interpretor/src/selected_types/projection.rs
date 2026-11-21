//! Projection lookup returns a retained body so the traversal can descend
//! into its result even when a nested lookup changes the execution catalogue.
use super::*;
use invocation::Body;

pub(super) trait Lookup<'p> {
    type Failure: From<ErrorKind>;

    fn resolve(&mut self, key: Key, fuel: &mut usize) -> Result<Body<'p>, Self::Failure>;
}

impl<'p> Lookup<'p> for &Values<'p> {
    type Failure = ErrorKind;

    fn resolve(&mut self, key: Key, _: &mut usize) -> Result<Body<'p>, ErrorKind> {
        self.get_body(&key).ok_or(ErrorKind::UnmaterializedDispatch)
    }
}

pub(super) enum Failure {
    Traversal(ErrorKind),
    Execution(Error),
}

impl From<ErrorKind> for Failure {
    fn from(kind: ErrorKind) -> Self {
        Self::Traversal(kind)
    }
}

pub(super) struct Observation<'vm, 'p> {
    vm: &'vm mut Interpreter<'p>,
    source: Option<SourceRange>,
    observed: HashMap<Key, Arc<Expr>>,
    // The VM owns the overall u64 allowance. The structural walk borrows a
    // usize-sized window between initializer executions.
    window: usize,
}

impl<'vm, 'p> Observation<'vm, 'p> {
    pub(super) fn run(
        vm: &'vm mut Interpreter<'p>,
        ty: &Term,
        source: Option<SourceRange>,
    ) -> Result<Term, Error> {
        let mut fuel = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut query = Self {
            vm,
            source,
            observed: HashMap::new(),
            window: fuel,
        };
        let result = walk(ty, &mut query, &mut fuel, 0);
        query.charge_traversal(fuel);
        result.map_err(|failure| match failure {
            Failure::Execution(error) => error,
            Failure::Traversal(kind) => Error { kind, source },
        })
    }

    fn charge_traversal(&mut self, remaining: usize) {
        self.vm.budget.steps -= (self.window - remaining) as u64;
    }
}

impl<'p> Lookup<'p> for Observation<'_, 'p> {
    type Failure = Failure;

    fn resolve(&mut self, key: Key, fuel: &mut usize) -> Result<Body<'p>, Failure> {
        if let Some(body) = self.observed.get(&key) {
            return Ok(Body::Owned(body.clone()));
        }
        if let Some(body) = self.vm.associated_values.get_body(&key) {
            return Ok(body);
        }

        let source = self.source.unwrap_or(SourceRange { start: 0, end: 0 });
        self.charge_traversal(*fuel);
        // The structural prefix stays live while user code executes. A fresh
        // stack segment is needed only if that prefix left little native stack.
        let result = stacker::maybe_grow(1024 * 1024, 4 * 1024 * 1024, || {
            self.vm.eval_associated_member(
                &key.0,
                &key.1,
                key.2,
                &Term::Primitive(Primitive::Type),
                source,
            )
        });
        self.window = usize::try_from(self.vm.budget.steps).unwrap_or(usize::MAX);
        *fuel = self.window;
        let value = result.map_err(Failure::Execution)?;
        if let Some(body) = self.vm.associated_values.get_body(&key) {
            return Ok(body);
        }
        let ty = match value {
            Flow::Value(Value::Type(ty)) => ty,
            Flow::Value(Value::TypeParameter(declaration)) => Term::Rigid {
                declaration,
                sort: nir::Sort::Type,
            },
            _ => {
                return Err(Failure::Execution(
                    self.vm.failure(ErrorKind::Type, Some(source)),
                ));
            }
        };
        let body = Arc::new(Expr {
            source,
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(ty),
        });
        self.observed.insert(key, body.clone());
        Ok(Body::Owned(body))
    }
}
