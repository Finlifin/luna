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

impl Request {
    pub(super) fn access(&self) -> &Expr {
        &self.access
    }

    pub(super) fn respond(self, selection: Result<AssociatedSelection, Error>) -> Response {
        Response {
            request: self,
            selection,
        }
    }
}

/// A returned selection still needs VM admission. Keep the exact request and
/// its observed result domain with the answer; neither is a catalogue entry.
/// This owner is linear and stays inside the invocation session.
pub(super) struct Response {
    request: Request,
    selection: Result<AssociatedSelection, Error>,
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

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(associated-response {} ", self.request)?;
        match &self.selection {
            Ok(selection) => write!(f, "{selection}")?,
            Err(error) => write!(f, "(failed {})", nir::QuotedDisplay(&error.to_string()))?,
        }
        f.write_str(")")
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
                Preparation::Request(request) => {
                    let response = self.resolve_associated_request(request);
                    self.accept_associated_response(response)?
                }
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
        self.prepare_associated_for_driver(interface, subject, member, ty, source, false)
    }

    pub(super) fn prepare_associated_for_driver(
        &mut self,
        interface: &InterfaceInstance,
        subject: &Term,
        member: DeclarationId,
        ty: &Term,
        source: SourceRange,
        external: bool,
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
            if !self.comptime_only
                || self.graph.is_some()
                || (!external && self.associated_resolver.is_none())
            {
                return Err(self.failure(ErrorKind::UnmaterializedDispatch, Some(source)));
            }
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

    pub(super) fn resolve_associated_request(&mut self, request: Request) -> Response {
        let source = request.access.source;
        let selection = (|| {
            let resolver = self
                .associated_resolver
                .filter(|_| self.comptime_only && self.graph.is_none())
                .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(source)))?;
            host_budget::checked(&mut self.budget.steps, Some(source), |remaining| {
                resolver(&request.access, remaining)
            })
        })();
        Response { request, selection }
    }

    pub(super) fn accept_associated_response(
        &mut self,
        response: Response,
    ) -> Result<Activation<'a>, Error> {
        let Response { request, selection } = response;
        let Request {
            access,
            key,
            result_type,
        } = request;
        let source = access.source;
        let selected = selection?;
        let (expression, initializing) = match selected {
            AssociatedSelection::Completed(value) => (value.into_expression(), false),
            AssociatedSelection::Initializer(initializer) => (initializer.into_expression(), true),
        };
        nir::copy_budget::charge_expression(&expression, &mut self.budget.steps)
            .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        let expression = Arc::new(expression);
        // Admit once before publication. Re-reading the catalogue and checking
        // again could execute result-domain Type programs a second time.
        let active = self.admit_associated_body(
            invocation::Body::Owned(expression.clone()),
            key.clone(),
            initializing,
            result_type,
            source,
        )?;
        let catalog = if initializing {
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
            return Err(self.failure(ErrorKind::CyclicAssociatedInitializer(key.2), Some(source)));
        }
        Ok(Activation {
            expression,
            result_type,
            key: initializing.then_some(key),
            source,
        })
    }
}
