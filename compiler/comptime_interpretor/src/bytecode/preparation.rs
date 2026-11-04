//! Owned call preparation, driven between bytecode slices by the execution session.
use super::*;

#[derive(Debug)]
pub(crate) enum Preparation {
    Method {
        destination: usize,
        method: Arc<comptime_compiler::methods::Method>,
    },
    Call {
        code: Arc<Program>,
        instruction: usize,
        dynamic_method: Option<comptime_compiler::methods::Method>,
    },
}

impl fmt::Display for Preparation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Method {
                destination,
                method,
            } => {
                write!(f, "(prepare-method (destination {destination}) {method})")
            }
            Self::Call {
                instruction,
                dynamic_method,
                ..
            } => {
                write!(
                    f,
                    "(prepare-call (instruction {instruction}) (dynamic-method"
                )?;
                if let Some(method) = dynamic_method {
                    write!(f, " {method}")?;
                }
                f.write_str("))")
            }
        }
    }
}

impl Interpreter<'_> {
    pub(crate) fn prepare_bytecode_request(
        &mut self,
        frame: &mut Frame,
        request: Preparation,
    ) -> Result<(), Error> {
        if frame.terminal || !matches!(frame.waiting, Some(Waiting::Preparation)) {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        match request {
            Preparation::Method {
                destination,
                method,
            } => {
                let value = self.bytecode_trait_method(method.view(), false)?;
                frame.slots[destination] = Some(value);
            }
            Preparation::Call {
                code,
                instruction,
                dynamic_method,
            } => {
                if !Arc::ptr_eq(&frame.code, &code) || frame.pc.checked_sub(1) != Some(instruction)
                {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, None));
                }
                self.prepare_bytecode_call(
                    frame,
                    &code.instructions()[instruction],
                    dynamic_method,
                )?;
            }
        }
        frame.waiting = None;
        Ok(())
    }

    fn prepare_bytecode_call(
        &mut self,
        frame: &mut Frame,
        instruction: &comptime_compiler::Instruction,
        dynamic_method: Option<comptime_compiler::methods::Method>,
    ) -> Result<(), Error> {
        let source = instruction.source;
        let missing = || Error {
            kind: ErrorKind::MissingBinding,
            source: Some(source),
        };
        let Op::BeginCall {
            call,
            callee,
            callee_source,
            normal_form,
            parameter_bindings,
            prepare_defaults,
            method,
        } = &instruction.op
        else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
        };
        let trait_defaults = if *prepare_defaults {
            dynamic_method
                .as_ref()
                .map(|method| self.trait_method_default_inputs(method.view()))
                .transpose()?
        } else {
            None
        };
        let method = method.as_deref().or(dynamic_method.as_ref());
        let (interface_program, interface_parameters) = if let Some(method) = method {
            self.interface_method_contract(method.view(), source)?
        } else {
            (Default::default(), None)
        };
        let callee = if let Some(method) = method {
            self.tick(Some(method.source))?;
            self.depth += 1;
            let value = self.bytecode_trait_method(method.view(), true);
            self.depth -= 1;
            value?
        } else {
            frame.slots[*callee].take().ok_or_else(missing)?
        };
        let callee = if normal_form.is_none() {
            self.resolve_invocation_identity(callee)?
        } else {
            callee
        };
        let (normal_head, parameters, count) = if let Some((ty, count)) = normal_form {
            if !self.has_type(&callee, ty) {
                return Err(self.failure(ErrorKind::Type, Some(*callee_source)));
            }
            let Value::Function(head) = &callee else {
                return Err(self.failure(ErrorKind::Type, Some(*callee_source)));
            };
            let metadata = self.normal_forms.get(head).ok_or_else(missing)?;
            if !parameter_bindings.is_empty() || *count != metadata.parameters.len() {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            (Some(*head), vec![], *count)
        } else {
            let parameters = self.call_parameters(&callee, source)?;
            let count = parameters.len();
            (None, parameters, count)
        };
        let preparation_parameters = interface_parameters.as_ref().unwrap_or(&parameters);
        if !parameter_bindings.is_empty()
            && (*parameter_bindings != *preparation_parameters || parameter_bindings.len() != count)
        {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        self.effect_charge(count, Some(source))?;
        frame.calls.insert(
            *call,
            PendingCall {
                callee,
                normal_head,
                trait_defaults,
                interface: interface_parameters.map(|parameters| (interface_program, parameters)),
                arguments: (0..count).map(|_| None).collect(),
                parameter_bindings: if *prepare_defaults {
                    parameter_bindings.clone()
                } else {
                    vec![]
                },
                defaults: Environment::new(),
                last_default: None,
            },
        );
        Ok(())
    }
}
