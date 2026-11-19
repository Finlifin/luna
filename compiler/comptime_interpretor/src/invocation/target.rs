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
    pub(super) fn admit_invocation(
        &mut self,
        value: Value,
        has_cleanup_context: bool,
    ) -> Result<Target<'a>, Error> {
        let target = match value {
            Value::Function(id) => Target::Named(self.named_target(id)?),
            Value::FunctionInstance {
                declaration,
                arguments,
            } => {
                let definition = self.named_target(declaration)?;
                self.check_instance_phase(definition)?;
                let source = definition.source;
                let frames = [self.ordinary_input_frame(&definition.signature, source)?];
                let inputs = self.admit_function_inputs(
                    &definition.type_parameters,
                    &arguments,
                    nir::DeclarationInputScope::ClosedInBody(&frames),
                    source,
                )?;
                let formation = (&definition.interface).into();
                self.check_callable_requirements(formation, &inputs, source)?;
                let signature = self.signature_from_admission(formation, &inputs, source)?;
                let substitution = self.query_admission(Some(source), |_, fuel| {
                    Ok(inputs.into_substitution().into_owned(fuel)?)
                })?;
                Target::Instance {
                    function: definition,
                    signature,
                    substitution,
                }
            }
            Value::IirCallable(handle) => Target::Graph(self.iir_target(&handle)?, handle),
            Value::Closure(closure) => Target::Closure(closure),
            Value::RuntimeTraitMethod(_) => {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
            _ if self.comptime_only => return Err(self.failure(ErrorKind::PhaseMismatch, None)),
            _ => return Err(self.failure(ErrorKind::Type, None)),
        };
        if self.comptime_only {
            let allowed = match &target {
                Target::Closure(c) => c.ty.is_comptime_callable(),
                Target::Named(f) | Target::Graph(f, _) | Target::Instance { function: f, .. } => {
                    f.phase == nir::program::FunctionPhase::Comptime
                }
            };
            if !allowed {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
        }
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
        Ok(target)
    }

    fn named_target(&self, id: DeclarationId) -> Result<&'a Function, Error> {
        if self.normal_forms.contains_key(&id) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.functions.definition(&id).ok_or_else(|| {
            self.failure(
                if self.comptime_only {
                    ErrorKind::PhaseMismatch
                } else {
                    ErrorKind::UnknownFunction
                },
                None,
            )
        })
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
