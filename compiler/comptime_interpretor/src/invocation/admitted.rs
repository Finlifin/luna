//! A source call after ordinary input and type-contract admission. The host can
//! inspect the bound interface before deciding whether to execute this frame.
use super::*;

pub struct AdmittedCall<'vm, 'world> {
    vm: &'vm mut Interpreter<'world>,
    activation: Option<Box<Activation<'world>>>,
    domains: Vec<Term>,
}

impl<'world> Interpreter<'world> {
    /// Begin one named comptime call. Report the shared balance even on failure.
    pub fn admit_source_call(
        &mut self,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        values: Vec<Value>,
        remaining: &mut usize,
    ) -> Result<AdmittedCall<'_, 'world>, Error> {
        self.budget.steps = self.budget.steps.min(*remaining as u64);
        let before = self.budget.steps;
        let mut domains = Vec::new();
        let prepared = (|| {
            if self.graph.is_some() || !self.comptime_only {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
            self.begin_named_invocation(declaration, arguments, values, Some(&mut domains))
        })();
        *remaining -= (before - self.budget.steps) as usize;
        Ok(AdmittedCall {
            vm: self,
            activation: Some(prepared?),
            domains,
        })
    }
}

impl<'world> AdmittedCall<'_, 'world> {
    fn frame(&self) -> &InvocationFrame<'world> {
        &self.activation.as_ref().expect("live admitted call").frame
    }
    pub fn domains(&self) -> &[Term] {
        &self.domains
    }
    pub fn result_type(&self) -> &Term {
        &self.frame().result_type
    }

    /// Borrow the arguments in declaration order while the admitted frame owns them.
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        let frame = self.frame();
        frame
            .target
            .declaration()
            .expect("named source call")
            .parameters
            .iter()
            .map(|id| frame.environment.get(id).expect("bound source argument"))
    }

    /// Resume the admitted activation with the balance left by the host.
    pub fn resume(self, remaining: &mut usize) -> Result<Value, Error> {
        self.with_allowance(remaining, Self::execute)
    }

    /// Deliver the value and whether it can be exported as immutable data of
    /// the bound result domain. Non-exportable values are still delivered.
    pub fn resume_for_export(self, remaining: &mut usize) -> Result<(Value, bool), Error> {
        self.with_allowance(remaining, |call| {
            let frame = &call.activation.as_ref().expect("live admitted call").frame;
            let domain = call.vm.query_admission(Some(frame.source), |_, fuel| {
                nir::copy_budget::clone_term(&frame.result_type, fuel)
                    .map_err(|_| nir::NominalAdmissionError::Budget)
            })?;
            let value = call.execute()?;
            if value.immutable_copy_work().is_none() {
                return Ok((value, false));
            }
            let exportable = match call.vm.validate_immutable_input(&value, &domain) {
                Ok(()) => true,
                Err(Error {
                    kind: ErrorKind::Type | ErrorKind::FunctionInputRejected(_),
                    ..
                }) => false,
                Err(error) => return Err(error),
            };
            Ok((value, exportable))
        })
    }

    fn execute(&mut self) -> Result<Value, Error> {
        let active = self.activation.take().expect("live admitted call");
        self.vm.run_activation(active)
    }

    fn with_allowance<R>(mut self, remaining: &mut usize, run: impl FnOnce(&mut Self) -> R) -> R {
        self.vm.budget.steps = self.vm.budget.steps.min(*remaining as u64);
        let before = self.vm.budget.steps;
        let result = run(&mut self);
        *remaining -= (before - self.vm.budget.steps) as usize;
        result
    }
}

impl Drop for AdmittedCall<'_, '_> {
    fn drop(&mut self) {
        if let Some(active) = self.activation.take() {
            self.vm.leave_invocation(active.saved);
        }
    }
}
impl fmt::Display for AdmittedCall<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(admitted-call (domains")?;
        for ty in &self.domains {
            write!(f, " {ty}")?;
        }
        write!(f, ") (result {}))", self.result_type())
    }
}
