//! An activation retains the definition that admitted it across contract steps.
use super::*;
use nir::program::{Function, FunctionDeclaration};

pub(super) enum Target<'a> {
    Named(&'a Function),
    Instance {
        function: &'a Function,
        signature: Term,
        substitution: nir::DeclarationSubstitution<'static>,
    },
    Graph(&'a Function, Arc<IirCallable>),
    Closure(Arc<Closure>),
}

impl<'a> Target<'a> {
    pub(super) fn declaration(&self) -> Option<&'a FunctionDeclaration> {
        match self {
            Self::Named(f) | Self::Instance { function: f, .. } | Self::Graph(f, _) => {
                Some(&f.interface)
            }
            Self::Closure(_) => None,
        }
    }

    pub(super) fn substitution(&self) -> Option<&nir::DeclarationSubstitution<'static>> {
        match self {
            Self::Instance { substitution, .. } => Some(substitution),
            _ => None,
        }
    }

    fn identity(&self) -> Option<(DeclarationId, &[Term])> {
        match self {
            Self::Named(f) => Some((f.declaration, &[])),
            Self::Instance {
                function,
                substitution,
                ..
            } => Some((function.declaration, substitution.arguments())),
            Self::Graph(_, handle) => Some((handle.key.declaration, &handle.key.arguments)),
            Self::Closure(_) => None,
        }
    }

    pub(super) fn cleanup_key(&self) -> Option<iir::InstanceKey> {
        match self {
            Self::Graph(_, handle) => Some(handle.key.clone()),
            _ => self
                .identity()
                .map(|(declaration, arguments)| iir::InstanceKey {
                    declaration,
                    arguments: arguments.into(),
                }),
        }
    }
}

impl fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(definition) => write!(f, "(function {})", definition.declaration),
            Self::Instance {
                function,
                substitution,
                ..
            } => {
                write!(f, "(function-instance {} (arguments", function.declaration)?;
                for input in substitution.arguments() {
                    write!(f, " {input}")?;
                }
                f.write_str("))")
            }
            Self::Graph(_, handle) => {
                write!(f, "(graph-instance {} {})", handle.position, handle.key)
            }
            Self::Closure(closure) => write!(
                f,
                "(closure (type {}) (source {} {}))",
                closure.ty, closure.body.source.start, closure.body.source.end
            ),
        }
    }
}

impl<'a> Interpreter<'a> {
    /// Resolve the explicit source entry without manufacturing a function-value expression.
    pub(super) fn named_entry_target(
        &mut self,
        declaration: DeclarationId,
        arguments: Vec<Term>,
    ) -> Result<Target<'a>, Error> {
        self.tick(None)?;
        let function = self.named_target(declaration)?;
        if arguments.len() != function.type_parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(function.source)));
        }
        if arguments.is_empty() {
            return Ok(Target::Named(function));
        }
        self.instance_target(function, arguments)
    }

    fn instance_target(
        &mut self,
        function: &'a Function,
        mut arguments: Vec<Term>,
    ) -> Result<Target<'a>, Error> {
        let source = function.source;
        // Both a direct call and a transported identity supply completed
        // terms. Describe them in the receiving VM before specializing code.
        let formation = nir::callable_source::Formation::from(&function.interface);
        let (signature, substitution) = self.query_descriptions(Some(source), |vm, fuel| {
            for term in &mut arguments {
                vm.inspect_term_description(term, fuel)?;
                *term = selected_types::view(term, &vm.associated_values, fuel)?;
            }
            let domains = function_inputs::input_domains(
                &vm.declaration_parameter_domains,
                formation.parameters,
                fuel,
            )
            .map_err(|_| ErrorKind::StepsExhausted)?;
            vm.with_admission_queries(|queries| {
                formation.admit_instance(&domains, arguments, fuel, queries)
            })
            .map_err(|failure| function_inputs::input_failure(formation.parameters, failure))
        })?;
        Ok(Target::Instance {
            function,
            signature,
            substitution,
        })
    }

    pub(super) fn invocation_target(&mut self, value: Value) -> Result<Target<'a>, Error> {
        Ok(match value {
            Value::Function(id) => Target::Named(self.named_target(id)?),
            Value::FunctionInstance {
                declaration,
                arguments,
            } => {
                let function = self.named_target(declaration)?;
                self.instance_target(function, arguments)?
            }
            Value::IirCallable(handle) => {
                let function = self.iir_target(&handle)?;
                self.check_instance_phase(function)?;
                Target::Graph(function, handle)
            }
            Value::Closure(closure) => {
                if self.comptime_only && !closure.ty.is_comptime_callable() {
                    return Err(self.failure(ErrorKind::PhaseMismatch, None));
                }
                Target::Closure(closure)
            }
            Value::RuntimeTraitMethod(_) => {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
            _ if self.comptime_only => return Err(self.failure(ErrorKind::PhaseMismatch, None)),
            _ => return Err(self.failure(ErrorKind::Type, None)),
        })
    }

    pub(super) fn check_invocation_cleanup(
        &mut self,
        target: &Target<'a>,
        has_cleanup_context: bool,
    ) -> Result<(), Error> {
        if let Some(schedule) = self.cleanup_schedule
            && !has_cleanup_context
            && let Some((declaration, arguments)) = target.identity()
        {
            for key in &schedule.borrowed_callable_functions {
                self.tick(None)?;
                if key.declaration == declaration && key.arguments.as_ref() == arguments {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
            if let Some(definition) = target.declaration()
                && matches!(definition.signature.callable_signature(), Term::Function {parameters,..} if parameters.iter().any(|p|matches!(p.ty.callable_signature(),Term::Function {..})))
            {
                return Err(self.failure(ErrorKind::InvalidCallPlan, None));
            }
        }
        Ok(())
    }

    fn named_target(&self, id: DeclarationId) -> Result<&'a Function, Error> {
        if self.normal_forms.contains_key(&id) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let function = self.functions.definition(&id).ok_or_else(|| {
            let kind = if self.functions.contains(&id) {
                ErrorKind::PhaseMismatch
            } else {
                ErrorKind::UnknownFunction
            };
            self.failure(kind, None)
        })?;
        self.check_instance_phase(function)?;
        Ok(function)
    }

    pub(super) fn check_instance_phase(
        &self,
        definition: &FunctionDeclaration,
    ) -> Result<(), Error> {
        if definition.phase == nir::program::FunctionPhase::NormalForm
            || (self.comptime_only && definition.phase != nir::program::FunctionPhase::Comptime)
        {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        Ok(())
    }
}
