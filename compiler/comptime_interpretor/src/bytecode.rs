//! Invocation-only execution of linear code. Runtime cleanup and mixed-stage
//! interpretation keep their existing paths. No opcode recursively evaluates NIR.
use super::*;
use comptime_compiler::{Op, Program};

struct Frame<'code, 'nir> {
    code: &'code Program<'nir>,
    environment: Environment,
    imported: Vec<(LocalId, usize)>,
    terminal: bool,
    pc: usize,
    slots: Vec<Option<Value>>,
    scopes: Vec<Vec<usize>>,
}

#[derive(Debug)]
enum Progress {
    Suspended,
    Complete(Flow),
}

impl fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(bytecode-frame (pc {}) (terminal {}) (registers",
            self.pc, self.terminal
        )?;
        for (slot, value) in self.slots.iter().enumerate() {
            if let Some(value) = value {
                write!(f, " (slot {slot} {value})")?;
            }
        }
        write!(f, ") (scopes")?;
        for scope in &self.scopes {
            write!(f, " (scope")?;
            for slot in scope {
                write!(f, " {slot}")?;
            }
            write!(f, ")")?;
        }
        write!(f, ") (imported")?;
        for (binding, slot) in &self.imported {
            write!(f, " ({binding} {slot})")?;
        }
        write!(f, ") (environment-bindings {}))", self.environment.len())
    }
}

impl Frame<'_, '_> {
    fn restore_environment(mut self) -> Environment {
        for (binding, slot) in self.imported {
            if let Some(value) = self.slots[slot].take() {
                self.environment.insert(binding, value);
            }
        }
        self.environment
    }
}

/// Only immutable package bodies are admitted. Neither invocation results nor
/// captured environments are stored here. A missing program records unsupported
/// lowering, so repeated calls do not repeatedly traverse a migration body.
#[derive(Default)]
pub(super) struct CodeCache<'a> {
    entries: BTreeMap<DeclarationId, Option<Arc<Program<'a>>>>,
    weight: usize,
}

impl<'a> CodeCache<'a> {
    fn insert(&mut self, id: DeclarationId, program: Option<Arc<Program<'a>>>) {
        const LIMIT: usize = 100_000;
        let weight = program.as_ref().map_or(1, |code| {
            1 + code.instructions().len() + code.locals().len() + code.registers()
        });
        if weight > LIMIT {
            return;
        }
        if self.weight + weight > LIMIT {
            self.entries.clear();
            self.weight = 0;
        }
        self.weight += weight;
        self.entries.insert(id, program);
    }
}

impl<'a> Interpreter<'a> {
    pub fn bytecode_invocations(&self) -> usize {
        self.bytecode_invocations
    }

    pub fn bytecode_compilations(&self) -> usize {
        self.bytecode_compilations
    }

    fn compile_body<'b>(&mut self, body: &'b Expr) -> Result<Option<Program<'b>>, Error> {
        self.bytecode_compilations += 1;
        match comptime_compiler::compile(body, &mut 100_000) {
            Ok(program) => Ok(Some(program)),
            Err(comptime_compiler::Error::Unsupported(_)) => Ok(None),
            Err(comptime_compiler::Error::Budget(source)) => {
                Err(self.failure(ErrorKind::StepsExhausted, Some(source)))
            }
            Err(comptime_compiler::Error::InvalidControl(source)) => {
                Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)))
            }
        }
    }

    pub(super) fn invocation_body(
        &mut self,
        body: &Expr,
        stable_code: Option<DeclarationId>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
            return self.eval(body, environment);
        }
        // Compilation has its own bounded work budget. Unsupported bodies are
        // selected before executing any instruction, never after partial work.
        if let Some(id) = stable_code {
            let program = if let Some(program) = self.bytecode_cache.entries.get(&id) {
                program.clone()
            } else {
                let definition = *self
                    .functions
                    .get(&id)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(body.source)))?;
                let program = self.compile_body(&definition.body)?.map(Arc::new);
                self.bytecode_cache.insert(id, program.clone());
                program
            };
            return if let Some(program) = program {
                self.bytecode_invocations += 1;
                self.execute_code(&program, environment)
            } else {
                self.eval(body, environment)
            };
        }
        let Some(program) = self.compile_body(body)? else {
            return self.eval(body, environment);
        };
        self.bytecode_invocations += 1;
        self.execute_code(&program, environment)
    }

    fn execute_code(
        &mut self,
        code: &Program<'_>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let mut frame = self.start_frame(code, environment)?;
        let result = loop {
            match self.resume_frame(&mut frame, 1024) {
                Ok(Progress::Suspended) => continue,
                Ok(Progress::Complete(flow)) => break Ok(flow),
                Err(error) => break Err(error),
            }
        };
        // Delivery contracts observe input mutations on both success and error.
        *environment = frame.restore_environment();
        result
    }

    fn start_frame<'code, 'nir>(
        &mut self,
        code: &'code Program<'nir>,
        environment: &mut Environment,
    ) -> Result<Frame<'code, 'nir>, Error> {
        self.effect_charge(
            code.registers(),
            code.instructions().first().map(|i| i.source),
        )?;
        let mut frame = Frame {
            code,
            environment: std::mem::take(environment),
            imported: Vec::new(),
            terminal: false,
            pc: 0,
            scopes: Vec::new(),
            slots: (0..code.registers()).map(|_| None).collect(),
        };
        for &(binding, slot) in code.locals() {
            if let Some(value) = frame.environment.remove(&binding) {
                frame.imported.push((binding, slot));
                frame.slots[slot] = Some(value);
            }
        }
        Ok(frame)
    }

    fn resume_frame(
        &mut self,
        frame: &mut Frame<'_, '_>,
        quantum: usize,
    ) -> Result<Progress, Error> {
        if frame.terminal {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        let result = self.run_frame(frame, quantum);
        // A failed instruction may have consumed operands or changed storage.
        // Such a frame is terminal; resumption must never replay that work.
        if !matches!(result, Ok(Progress::Suspended)) {
            frame.terminal = true;
        }
        result
    }

    fn run_frame(&mut self, frame: &mut Frame<'_, '_>, quantum: usize) -> Result<Progress, Error> {
        let code = frame.code;
        for _ in 0..quantum {
            let instruction = code
                .instructions()
                .get(frame.pc)
                .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))?;
            let source = instruction.source;
            self.tick(Some(source))?;
            frame.pc += 1;
            let missing = || Error {
                kind: ErrorKind::MissingBinding,
                source: Some(source),
            };
            match &instruction.op {
                Op::PrepareIntrinsic {
                    tag,
                    binding,
                    inputs,
                    output,
                } => {
                    let binding = binding
                        .filter(|binding| binding.is_comptime())
                        .ok_or_else(|| self.failure(ErrorKind::Unsupported, Some(source)))?;
                    binding
                        .validate_types(*tag, inputs.iter().copied(), output)
                        .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                }
                Op::Intrinsic {
                    destination,
                    binding,
                    inputs,
                } => {
                    let binding = binding
                        .ok_or_else(|| self.failure(ErrorKind::Unsupported, Some(source)))?;
                    let values: Vec<_> = inputs
                        .iter()
                        .map(|slot| frame.slots[*slot].take().ok_or_else(missing))
                        .collect::<Result<_, _>>()?;
                    frame.slots[*destination] =
                        Some(self.scalar_math_values(binding, &values, source)?);
                }
                Op::Constant { destination, value } => {
                    let flow = match value.kind {
                        K::Literal(_) => self.eval_literal(value, &mut frame.environment)?,
                        _ => self.eval_type_value(value, &mut frame.environment)?,
                    };
                    let Flow::Value(value) = flow else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::EnterScope => frame.scopes.push(Vec::new()),
                Op::LeaveScope => {
                    let locals = frame
                        .scopes
                        .pop()
                        .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, Some(source)))?;
                    for slot in locals {
                        frame.slots[slot] = None;
                    }
                }
                Op::Unwind(depth) => {
                    if *depth > frame.scopes.len() {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    }
                    while frame.scopes.len() > *depth {
                        for slot in frame.scopes.pop().unwrap() {
                            frame.slots[slot] = None;
                        }
                    }
                }
                Op::PrepareScalar(scalar) => {
                    self.prepare_scalar(*scalar, source)?;
                }
                Op::CheckScalarOperand { scalar, value } => {
                    let plan = self.prepare_scalar(*scalar, source)?;
                    self.scalar_operand(
                        plan,
                        frame.slots[*value].as_ref().ok_or_else(missing)?,
                        source,
                    )?;
                }
                Op::Scalar {
                    destination,
                    scalar,
                    left,
                    right,
                } => {
                    let plan = self.prepare_scalar(*scalar, source)?;
                    let a = frame.slots[*left].take().ok_or_else(missing)?;
                    let b = right
                        .map(|slot| frame.slots[slot].take().ok_or_else(missing))
                        .transpose()?;
                    frame.slots[*destination] =
                        Some(self.scalar_values(*scalar, plan, a, b, source)?);
                }
                Op::Unit(dst) => frame.slots[*dst] = Some(Value::Unit),
                Op::Read { destination, local } => {
                    let input = frame.slots[*local].as_ref().ok_or_else(missing)?;
                    let value = self.invocation_slot_value(input, Some(source))?;
                    frame.slots[*destination] = Some(value);
                }
                Op::Store {
                    local,
                    value,
                    initialize,
                } => {
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    if *initialize {
                        if frame.slots[*local].is_none() {
                            if let Some(scope) = frame.scopes.last_mut() {
                                scope.push(*local);
                            }
                        }
                        frame.slots[*local] = Some(value);
                    } else {
                        let target = frame.slots[*local].as_mut().ok_or_else(missing)?;
                        match target {
                            Value::ComptimeReference(reference) => reference
                                .replace(value, &mut self.budget.steps)
                                .map_err(|kind| self.failure(kind, Some(source)))?,
                            Value::Storage(storage) => {
                                value_reference::ValueReference::root(storage.clone())
                                    .replace(value, &mut self.budget.steps)
                                    .map_err(|kind| self.failure(kind, Some(source)))?
                            }
                            target => *target = value,
                        }
                    }
                }
                Op::Not {
                    destination,
                    value,
                    ty,
                } => {
                    let value = Self::concrete_value(
                        frame.slots[*value].take().ok_or_else(missing)?,
                        source,
                    )?;
                    let result = match value {
                        Value::Bool(value) => Value::Bool(!value),
                        Value::BitVec(value)
                            if **ty == Term::Primitive(Primitive::BitVec(value.width())) =>
                        {
                            Value::BitVec(self.bit_operation(source, |budget| value.not(budget))?)
                        }
                        _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                    };
                    frame.slots[*destination] = Some(result);
                }
                Op::Jump(target) => frame.pc = *target,
                Op::Branch {
                    condition,
                    when,
                    target,
                } => match frame.slots[*condition].as_ref().ok_or_else(missing)? {
                    Value::Bool(value) => {
                        if value == when {
                            frame.pc = *target;
                        }
                    }
                    Value::AbstractValue { .. } => {
                        return Err(self.failure(ErrorKind::AbstractValueUnresolved, Some(source)));
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                },
                Op::Finish { value, explicit } => {
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    return Ok(Progress::Complete(if *explicit {
                        Flow::Return(value)
                    } else {
                        Flow::Value(value)
                    }));
                }
            }
        }
        Ok(Progress::Suspended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_slices_preserve_values_scopes_and_work_charges() {
        let package = Package::default();
        let input = LocalId { owner: 1, index: 0 };
        let temporary = LocalId { owner: 1, index: 1 };
        let unrelated = LocalId { owner: 2, index: 0 };
        let expr = |primitive, kind| Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(primitive),
            kind,
        };
        let mut statements = vec![expr(
            Primitive::Void,
            K::Let {
                binding: temporary,
                value: Box::new(expr(Primitive::Void, K::Unit)),
            },
        )];
        for _ in 0..3 {
            statements.push(expr(
                Primitive::Void,
                K::Assign {
                    place: nir::program::Place {
                        root: input,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        Primitive::Bool,
                        K::Not(Box::new(expr(Primitive::Bool, K::Local(input)))),
                    )),
                },
            ));
        }
        let body = expr(
            Primitive::Bool,
            K::Block {
                statements,
                result: Some(Box::new(expr(Primitive::Bool, K::Local(input)))),
            },
        );
        let code = comptime_compiler::compile(&body, &mut 1000).unwrap();
        let mut remaining = None;
        for quantum in [1, 2, 7, 1024] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            environment.insert(input, Value::Bool(true));
            environment.insert(unrelated, Value::Bool(true));
            let mut frame = vm.start_frame(&code, &mut environment).unwrap();
            assert!(environment.is_empty());
            let initial = frame.to_string();
            let before = vm.budget.steps;
            assert!(matches!(
                vm.resume_frame(&mut frame, 0).unwrap(),
                Progress::Suspended
            ));
            assert_eq!(frame.to_string(), initial);
            assert_eq!(vm.budget.steps, before);
            loop {
                match vm.resume_frame(&mut frame, quantum).unwrap() {
                    Progress::Suspended => assert!(!frame.terminal),
                    Progress::Complete(Flow::Value(Value::Bool(false))) => break,
                    progress => panic!("unexpected progress: {progress:?}"),
                }
            }
            assert!(frame.scopes.is_empty());
            assert!(frame.to_string().contains("(terminal true)"));
            let before = vm.budget.steps;
            assert_eq!(
                vm.resume_frame(&mut frame, quantum).unwrap_err().kind,
                ErrorKind::InvalidControlFlow
            );
            assert_eq!(vm.budget.steps, before);
            environment = frame.restore_environment();
            assert_eq!(environment.get(&input), Some(&Value::Bool(false)));
            assert_eq!(environment.get(&unrelated), Some(&Value::Bool(true)));
            assert!(!environment.contains_key(&temporary));
            if let Some(remaining) = remaining {
                assert_eq!(vm.budget.steps, remaining);
            }
            remaining = Some(vm.budget.steps);
        }
    }

    #[test]
    fn unbound_intrinsic_rejects_before_operand_effects() {
        let package = Package::default();
        let trace = LocalId { owner: 1, index: 0 };
        let source = SourceRange { start: 1, end: 2 };
        let body = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Intrinsic {
                tag: nir::intrinsics::Tag {
                    family: DeclarationId {
                        package: 1,
                        local: 0,
                    },
                    variant: 0,
                },
                binding: None,
                inputs: vec![Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::Assign {
                        place: nir::program::Place {
                            root: trace,
                            fields: vec![],
                        },
                        value: Box::new(Expr {
                            source,
                            ty: Term::Primitive(Primitive::Bool),
                            kind: K::Literal(Literal::Bool(true)),
                        }),
                    },
                }],
            },
        };
        let code = comptime_compiler::compile(&body, &mut 100).unwrap();
        let dump = code.to_string();
        assert!(dump.contains("(prepare-intrinsic (intrinsic-tag"));
        assert!(dump.contains("(inputs (primitive void)) (output (primitive void))"));
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            environment.insert(trace, Value::Bool(false));
            let result = if compiled {
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            assert_eq!(result.unwrap_err().kind, ErrorKind::Unsupported);
            assert_eq!(environment.get(&trace), Some(&Value::Bool(false)));
        }
    }

    #[test]
    fn unsupported_code_cache_is_bounded_and_eviction_allows_reinsertion() {
        let mut cache = CodeCache::default();
        for local in 0..=100_000 {
            cache.insert(DeclarationId { package: 1, local }, None);
        }
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.weight, 1);
        let first = DeclarationId {
            package: 1,
            local: 0,
        };
        assert!(!cache.entries.contains_key(&first));
        cache.insert(first, None);
        assert!(cache.entries.contains_key(&first));
        assert_eq!(cache.weight, 2);
    }

    #[test]
    fn scalar_failures_preserve_signature_and_operand_check_order() {
        let package = Package::default();
        let trace = LocalId { owner: 1, index: 0 };
        let expr = |primitive, kind| Expr {
            ty: Term::Primitive(primitive),
            kind,
            source: SourceRange { start: 1, end: 2 },
        };
        // Malformed typed values exercise the execution boundary independently
        // of frontend admission. Integer operands must fail before right-side
        // effects, while scalar comparisons evaluate both sides before checking.
        for (comparison, invalid_signature, changed) in [
            (false, false, false),
            (true, false, true),
            (true, true, false),
        ] {
            let domain = if comparison {
                Primitive::Bool
            } else {
                Primitive::Integer
            };
            let left = Box::new(expr(domain, K::Unit));
            let right = Box::new(expr(
                domain,
                K::Block {
                    statements: vec![expr(
                        Primitive::Void,
                        K::Assign {
                            place: nir::program::Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                        },
                    )],
                    result: Some(Box::new(expr(domain, K::Unit))),
                },
            ));
            let body = expr(
                if invalid_signature {
                    Primitive::Void
                } else {
                    domain
                },
                if comparison {
                    K::ScalarEqual(left, right)
                } else {
                    K::IntegerBinary {
                        op: nir::program::IntegerOp::Add,
                        left,
                        right,
                    }
                },
            );
            let code = comptime_compiler::compile(&body, &mut 1000).unwrap();
            for compiled in [false, true] {
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut environment = Environment::new();
                environment.insert(trace, Value::Bool(false));
                let result = if compiled {
                    vm.execute_code(&code, &mut environment)
                } else {
                    vm.eval(&body, &mut environment)
                };
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                assert_eq!(environment.get(&trace), Some(&Value::Bool(changed)));
            }
        }
    }

    #[test]
    fn failed_frame_preserves_prior_writes_and_does_not_leak_locals() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let input = LocalId {
            owner: 1,
            index: 1000,
        };
        let temporary = LocalId {
            owner: 2,
            index: u32::MAX,
        };
        let expr = |ty, kind| Expr {
            ty,
            kind,
            source: SourceRange { start: 1, end: 2 },
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let void = Term::Primitive(Primitive::Void);
        let body = expr(
            boolean.clone(),
            K::Block {
                statements: vec![
                    expr(
                        void.clone(),
                        K::Assign {
                            place: nir::program::Place {
                                root: input,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                boolean.clone(),
                                K::Literal(Literal::Bool(false)),
                            )),
                        },
                    ),
                    expr(
                        void.clone(),
                        K::Let {
                            binding: temporary,
                            value: Box::new(expr(void.clone(), K::Unit)),
                        },
                    ),
                ],
                // Deliberately malformed typed input probes the engine's error
                // boundary; source elaboration rejects this operation earlier.
                result: Some(Box::new(expr(
                    boolean.clone(),
                    K::Not(Box::new(expr(void, K::Unit))),
                ))),
            },
        );
        let code = comptime_compiler::compile(&body, &mut 100).unwrap();
        assert!(
            code.registers() < 16,
            "sparse identities must not size the frame"
        );
        let mut environment = Environment::new();
        environment.insert(input, Value::Bool(true));
        let mut frame = vm.start_frame(&code, &mut environment).unwrap();
        let failure = loop {
            match vm.resume_frame(&mut frame, 1) {
                Ok(Progress::Suspended) => continue,
                Ok(Progress::Complete(_)) => panic!("malformed operation completed"),
                Err(error) => break error,
            }
        };
        assert_eq!(failure.kind, ErrorKind::Type);
        let stopped = frame.to_string();
        assert_eq!(
            vm.resume_frame(&mut frame, 1).unwrap_err().kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(frame.to_string(), stopped);
        environment = frame.restore_environment();
        assert_eq!(environment.get(&input), Some(&Value::Bool(false)));
        assert!(!environment.contains_key(&temporary));
        let recovered = expr(boolean, K::Local(input));
        let code = comptime_compiler::compile(&recovered, &mut 100).unwrap();
        assert!(matches!(
            vm.execute_code(&code, &mut environment).unwrap(),
            Flow::Value(Value::Bool(false))
        ));
    }
}
