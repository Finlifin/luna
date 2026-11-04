//! Associated selection requests and admitted initialization are distinct states.
//! Requests are owned by the current VM driver, never by its code cache.
use super::*;

type Key = (InterfaceInstance, Term, DeclarationId);

pub(super) enum Preparation<'a> {
    Ready(Activation<'a>),
    Request(Request),
}

/// Prepared inputs, not permission to execute or publish a resolver response.
pub(super) struct Request {
    access: Expr,
    key: Key,
    result_type: Term,
}

pub(super) struct Activation<'a> {
    pub(super) expression: invocation::Body<'a>,
    result_type: Term,
    key: Option<Key>,
    source: SourceRange,
}

impl Activation<'_> {
    pub(super) fn complete(
        self,
        vm: &mut Interpreter<'_>,
        result: Result<Flow, Error>,
    ) -> Result<Flow, Error> {
        if let Some(key) = self.key {
            vm.active_associated_initializers.remove(&key);
        }
        let Flow::Value(value) = result? else {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.source)));
        };
        vm.check_value_type(&value, &self.result_type, Some(self.source))?;
        Ok(Flow::Value(value))
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-request (result {}) {})",
            self.result_type, self.access
        )
    }
}

impl fmt::Display for Activation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-activation (source {}) (result {}) (initializing",
            self.source, self.result_type
        )?;
        if let Some((interface, subject, member)) = &self.key {
            write!(f, " {interface} {subject} {member}")?;
        }
        write!(f, ") {})", self.expression.expression())
    }
}

impl<'a> Interpreter<'a> {
    #[inline(never)]
    pub(super) fn eval_associated_value(&mut self, expr: &Expr) -> Result<Flow, Error> {
        let K::AssociatedValue {
            interface,
            subject,
            member,
        } = &expr.kind
        else {
            unreachable!()
        };
        let prepared =
            self.prepare_associated_value(interface, subject, *member, &expr.ty, expr.source)?;
        if !self.bytecode_invocation_available() {
            let active = match prepared {
                Preparation::Ready(active) => active,
                Preparation::Request(request) => self.resolve_associated_request(request)?,
            };
            // Keep fixed tree recursion out of the driver's native stack frame.
            // Initializers never inherit the caller's lexical environment.
            let result = self.eval(active.expression.expression(), &mut Environment::default());
            return active.complete(self, result);
        }
        self.run_associated_preparation(prepared)
    }

    pub(super) fn prepare_associated_value(
        &mut self,
        interface: &InterfaceInstance,
        subject: &Term,
        member: DeclarationId,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Preparation<'a>, Error> {
        let selected_subject = self.selected_type_view(subject, Some(source))?;
        let selected_arguments = interface
            .arguments
            .iter()
            .map(|argument| self.selected_type_view(argument, Some(source)))
            .collect::<Result<Vec<_>, _>>()?;
        let key = (
            InterfaceInstance {
                declaration: interface.declaration,
                arguments: selected_arguments,
            },
            selected_subject,
            member,
        );
        // Keep this observation through selection and execution; returning from
        // the initializer must not replay its associated Type programs.
        let result_type = self.selected_type_view(ty, Some(source))?;
        if !self.associated_values.contains_key(&key)
            && !self.associated_initializers.contains_key(&key)
        {
            self.associated_resolver
                .filter(|_| self.comptime_only && self.graph.is_none())
                .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
            self.charge_copy_work(Some(source), |fuel| {
                for term in std::iter::once(&key.1)
                    .chain(&key.0.arguments)
                    .chain(std::iter::once(ty))
                {
                    nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
                }
                Ok(())
            })?;
            let access = Expr {
                source,
                ty: ty.clone(),
                kind: K::AssociatedValue {
                    interface: key.0.clone(),
                    subject: key.1.clone(),
                    member,
                },
            };
            return Ok(Preparation::Request(Request {
                access,
                key,
                result_type,
            }));
        }
        let selected = self
            .associated_values
            .get_body(&key)
            .or_else(|| {
                self.comptime_only
                    .then(|| self.associated_initializers.get_body(&key))
                    .flatten()
            })
            .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
        let initializing = self.associated_initializers.contains_key(&key);
        self.admit_associated_body(selected, key, initializing, result_type, source)
            .map(Preparation::Ready)
    }

    pub(super) fn resolve_associated_request(
        &mut self,
        request: Request,
    ) -> Result<Activation<'a>, Error> {
        let Request {
            access,
            key,
            result_type,
        } = request;
        let source = access.source;
        let resolver = self
            .associated_resolver
            .filter(|_| self.comptime_only && self.graph.is_none())
            .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
        let selected = host_budget::checked(&mut self.budget.steps, Some(source), |remaining| {
            resolver(&access, remaining)
        })?;
        let expression = selected.initializer.into_expression();
        nir::copy_budget::charge_expression(&expression, &mut self.budget.steps)
            .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        let expression = Arc::new(expression);
        // Admit once before publication. Re-reading the catalogue and checking
        // again could execute result-domain Type programs a second time.
        let active = self.admit_associated_body(
            invocation::Body::Owned(expression.clone()),
            key.clone(),
            selected.requires_invocation,
            result_type,
            source,
        )?;
        let catalog = if selected.requires_invocation {
            &mut self.associated_initializers
        } else {
            &mut self.associated_values
        };
        catalog.insert_owned(key, expression);
        Ok(active)
    }

    fn admit_associated_body(
        &mut self,
        expression: invocation::Body<'a>,
        key: Key,
        initializing: bool,
        result_type: Term,
        source: SourceRange,
    ) -> Result<Activation<'a>, Error> {
        if self.selected_type_view(&expression.expression().ty, Some(source))? != result_type {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if initializing && !self.active_associated_initializers.insert(key.clone()) {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
        }
        Ok(Activation {
            expression,
            result_type,
            key: initializing.then_some(key),
            source,
        })
    }
}
