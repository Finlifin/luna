//! Invocation-only execution of linear code. Runtime cleanup and mixed-stage
//! interpretation keep their existing paths. No opcode recursively evaluates NIR.
use super::*;
use comptime_compiler::{Op, Program};

pub(super) struct Frame {
    code: Arc<Program>,
    environment: Environment,
    imported: Vec<(LocalId, usize)>,
    terminal: bool,
    waiting: Option<Waiting>,
    calls: BTreeMap<usize, PendingCall>,
    normal_types: BTreeMap<usize, super::normal_types::Inputs>,
    staged: Option<Box<staged_bytecode::State>>,
    pc: usize,
    slots: Vec<Option<Value>>,
    scopes: Vec<Vec<usize>>,
}

enum Waiting {
    Call(usize),
    Default { call: usize, destination: usize },
}

struct PendingCall {
    callee: Value,
    arguments: Vec<Option<Value>>,
    parameter_bindings: Vec<LocalId>,
    defaults: Environment,
    last_default: Option<usize>,
}

#[derive(Debug)]
pub(super) enum Progress {
    Default {
        expression: Arc<Expr>,
        environment: Environment,
    },
    Call {
        callee: Value,
        arguments: Vec<Value>,
    },
    Suspended,
    Complete(Flow),
}

impl fmt::Display for Frame {
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
        write!(f, ") (waiting")?;
        match &self.waiting {
            Some(Waiting::Call(destination)) => write!(f, " (call {destination})")?,
            Some(Waiting::Default { call, destination }) => {
                write!(f, " (default {call} {destination})")?
            }
            None => {}
        }
        write!(f, ")")?;
        if let Some(staged) = &self.staged {
            write!(f, " {staged}")?;
        }
        write!(f, " (normal-types")?;
        for (slot, inputs) in &self.normal_types {
            write!(f, " (%{slot} {inputs})")?;
        }
        write!(f, ") (calls")?;
        for (id, call) in &self.calls {
            write!(f, " ({id} (callee {}) (arguments", call.callee)?;
            for value in &call.arguments {
                match value {
                    Some(value) => write!(f, " (ready {value})")?,
                    None => write!(f, " (pending)")?,
                }
            }
            write!(f, ") (defaults")?;
            for (binding, value) in &call.defaults {
                write!(f, " ({binding} {value})")?;
            }
            write!(f, ") (last-default")?;
            if let Some(parameter) = call.last_default {
                write!(f, " {parameter}")?;
            }
            write!(f, "))")?;
        }
        write!(f, ") (environment-bindings {}))", self.environment.len())
    }
}

pub(super) struct InputFrame {
    pub code: Frame,
    pub finish: InputFinish,
}
pub(super) enum InputFinish {
    Ready,
    Preparation {
        exports: Vec<(LocalId, usize)>,
        source: SourceRange,
    },
    Staged {
        shape: comptime_compiler::normal_types::Shape,
        depth: usize,
        source: SourceRange,
    },
}
pub(super) struct InputResult {
    pub flow: Result<Flow, Error>,
    pub environment: Environment,
    pub staged: Option<(
        comptime_compiler::normal_types::Shape,
        normal_inputs::StagedNormalInput,
    )>,
}
impl fmt::Display for InputFinish {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => write!(f, "(input-finish ready)"),
            Self::Preparation { exports, source } => {
                write!(
                    f,
                    "(input-finish preparation (source {} {}) (exports",
                    source.start, source.end
                )?;
                for (binding, slot) in exports {
                    write!(f, " ({binding} %{slot})")?;
                }
                write!(f, "))")
            }
            Self::Staged {
                shape,
                depth,
                source,
            } => write!(
                f,
                "(input-finish staged (depth {depth}) (source {} {}) {shape})",
                source.start, source.end
            ),
        }
    }
}
impl fmt::Display for InputFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(input-frame {} {})", self.finish, self.code)
    }
}
impl InputFinish {
    pub fn complete(
        self,
        vm: &mut Interpreter<'_>,
        mut code: Frame,
        mut flow: Result<Flow, Error>,
    ) -> InputResult {
        let mut staged = None;
        match self {
            Self::Ready => {}
            Self::Preparation { exports, source } => {
                if matches!(&flow, Ok(Flow::Value(_))) {
                    if exports.iter().any(|(_, slot)| code.slots[*slot].is_none()) {
                        flow = Err(vm.failure(ErrorKind::MissingBinding, Some(source)));
                    } else {
                        code.imported.extend(exports);
                    }
                }
            }
            Self::Staged {
                shape,
                depth,
                source,
            } => {
                vm.depth = depth;
                if matches!(&flow, Ok(Flow::Value(_))) {
                    match code
                        .staged
                        .as_mut()
                        .and_then(|state| state.completed.take())
                    {
                        Some(value) => staged = Some((shape, value)),
                        None => flow = Err(vm.failure(ErrorKind::InvalidControlFlow, Some(source))),
                    }
                }
            }
        }
        InputResult {
            flow,
            environment: code.restore_environment(),
            staged,
        }
    }
}
impl InputFrame {
    fn run(mut self, vm: &mut Interpreter<'_>) -> InputResult {
        let flow = vm.run_fragment(&mut self.code);
        self.finish.complete(vm, self.code, flow)
    }
}

impl Frame {
    pub(super) fn deliver(&mut self, value: Value) -> Result<(), Error> {
        let Some(Waiting::Call(destination)) = self.waiting else {
            return Err(Error {
                kind: ErrorKind::InvalidControlFlow,
                source: None,
            });
        };
        self.waiting = None;
        self.slots[destination] = Some(value);
        Ok(())
    }

    pub(super) fn receive(
        &mut self,
        result: Result<Flow, Error>,
        defaults: Option<Environment>,
    ) -> Result<Option<Flow>, Error> {
        if let Some(environment) = defaults {
            let Some(Waiting::Default { call, destination }) = self.waiting else {
                return Err(Error {
                    kind: ErrorKind::InvalidControlFlow,
                    source: None,
                });
            };
            let pending = self.calls.get_mut(&call).ok_or(Error {
                kind: ErrorKind::InvalidControlFlow,
                source: None,
            })?;
            pending.defaults = environment;
            self.waiting = Some(Waiting::Call(destination));
        }
        match result? {
            Flow::Value(value) => {
                self.deliver(value)?;
                Ok(None)
            }
            flow => {
                self.waiting = None;
                Ok(Some(flow))
            }
        }
    }

    pub(super) fn restore_environment(mut self) -> Environment {
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
pub(super) struct CodeCache {
    entries: BTreeMap<DeclarationId, Option<Arc<Program>>>,
    weight: usize,
}

impl CodeCache {
    fn insert(&mut self, id: DeclarationId, program: Option<Arc<Program>>) {
        const LIMIT: usize = 100_000;
        let weight = program.as_ref().map_or(1, |code| {
            1 + code.storage_work() + code.locals().len() + code.registers()
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

    pub fn bytecode_default_evaluations(&self) -> usize {
        self.bytecode_default_evaluations
    }

    pub fn bytecode_compilations(&self) -> usize {
        self.bytecode_compilations
    }

    /// Actual compilation work charged to invocation budgets, including failures.
    pub fn bytecode_compilation_steps(&self) -> u64 {
        self.bytecode_compilation_steps
    }

    fn compile_body(&mut self, body: &Expr) -> Result<Option<Program>, Error> {
        self.compile_bytecode(|fuel| comptime_compiler::compile(body, fuel))
    }

    fn compile_bytecode<T>(
        &mut self,
        compile: impl FnOnce(&mut usize) -> Result<T, comptime_compiler::Error>,
    ) -> Result<Option<T>, Error> {
        self.bytecode_compilations += 1;
        // Compilation is actual invocation work, including unsuccessful probes.
        // Use the caller's remaining budget rather than an unrelated fixed cap;
        // cache hits bypass this function and do not pay this work again.
        let before = self.budget.steps;
        let available = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = available;
        let result = compile(&mut remaining);
        self.budget.steps -= (available - remaining) as u64;
        self.bytecode_compilation_steps = self
            .bytecode_compilation_steps
            .saturating_add(before - self.budget.steps);
        match result {
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

    pub(super) fn invocation_code(
        &mut self,
        body: &Expr,
        stable_code: Option<DeclarationId>,
    ) -> Result<Option<Arc<Program>>, Error> {
        if !self.comptime_only
            || self.cleanup_schedule.is_some()
            || self.graph.is_some()
            || !self.effect_state.environment.is_empty()
        {
            return Ok(None);
        }
        // Decide the execution route before any instruction has run.
        if let Some(id) = stable_code {
            if let Some(program) = self.bytecode_cache.entries.get(&id) {
                return Ok(program.clone());
            }
            // The invocation boundary supplies either immutable package code
            // or a validated input-independent prepared body of that declaration.
            let program = self.compile_body(body)?.map(Arc::new);
            self.bytecode_cache.insert(id, program.clone());
            Ok(program)
        } else {
            Ok(self.compile_body(body)?.map(Arc::new))
        }
    }

    #[cfg(test)]
    fn execute_code(
        &mut self,
        code: &Arc<Program>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let mut frame = self.start_frame(code, environment)?;
        let result = self.run_fragment(&mut frame);
        *environment = frame.restore_environment();
        result
    }

    /// Staged input fragments actually started, including failed evaluations.
    pub fn bytecode_staged_inputs(&self) -> usize {
        self.bytecode_staged_inputs
    }

    fn contextual_code_available(&self) -> bool {
        self.comptime_only
            && self.cleanup_schedule.is_none()
            && self.graph.is_none()
            && self.effect_state.environment.is_empty()
    }

    pub(super) fn start_staged_input(
        &mut self,
        input: &nir::program::NormalFormArgument,
        environment: &mut Environment,
        depth: usize,
    ) -> Result<Option<InputFrame>, Error> {
        if !self.contextual_code_available() || depth != 0 {
            return Ok(None);
        }
        let Some((program, shape)) =
            self.compile_bytecode(|fuel| comptime_compiler::compile_staged_input(input, fuel))?
        else {
            return Ok(None);
        };
        let code = self.start_frame(&Arc::new(program), environment)?;
        self.bytecode_staged_inputs += 1;
        Ok(Some(InputFrame {
            code,
            finish: InputFinish::Staged {
                shape,
                depth: self.depth,
                source: input.source(),
            },
        }))
    }

    pub(super) fn try_stage_bytecode(
        &mut self,
        input: &nir::program::NormalFormArgument,
        environment: &mut Environment,
        depth: usize,
    ) -> Result<Option<std::ops::ControlFlow<Flow, normal_inputs::StagedNormalInput>>, Error> {
        let Some(frame) = self.start_staged_input(input, environment, depth)? else {
            return Ok(None);
        };
        let result = frame.run(self);
        *environment = result.environment;
        Ok(Some(match result.flow? {
            Flow::Value(_) => {
                let (shape, mut staged) = result.staged.expect("completed staged fragment");
                self.rekey_staged_input(input, &shape, &mut staged)?;
                std::ops::ControlFlow::Continue(staged)
            }
            flow => std::ops::ControlFlow::Break(flow),
        }))
    }

    pub(super) fn start_contextual_input(
        &mut self,
        input: &nir::program::NormalFormArgument,
        expected: &Term,
        environment: &mut Environment,
    ) -> Result<Option<InputFrame>, Error> {
        if !self.contextual_code_available() {
            return Ok(None);
        }
        let Some(program) = self.compile_bytecode(|fuel| {
            comptime_compiler::compile_contextual_input(input, expected, fuel)
        })?
        else {
            return Ok(None);
        };
        let code = self.start_frame(&Arc::new(program), environment)?;
        self.bytecode_contextual_inputs += 1;
        Ok(Some(InputFrame {
            code,
            finish: InputFinish::Ready,
        }))
    }

    pub(super) fn interpret_contextual_input(
        &mut self,
        input: &nir::program::NormalFormArgument,
        expected: &Term,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let mut literals = BTreeMap::new();
        self.prepare_normal_argument(input, expected, &mut literals, 0)?;
        self.eval_normal_argument(input, &mut literals, environment)
    }

    pub(super) fn evaluate_contextual_argument(
        &mut self,
        input: &nir::program::NormalFormArgument,
        expected: &Term,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if let Some(frame) = self.start_contextual_input(input, expected, environment)? {
            let result = frame.run(self);
            *environment = result.environment;
            return result.flow;
        }
        self.interpret_contextual_input(input, expected, environment)
    }

    pub fn bytecode_contextual_inputs(&self) -> usize {
        self.bytecode_contextual_inputs
    }

    pub(super) fn start_input_preparation(
        &mut self,
        preparation: &nir::program::NormalFormPreparation,
        environment: &mut Environment,
    ) -> Result<Option<InputFrame>, Error> {
        let source = preparation.expression.source;
        if let K::LetPattern {
            pattern: nir::program::Pattern::Tuple(fields),
            ..
        } = &preparation.expression.kind
        {
            self.effect_charge(fields.len(), Some(source))?;
        }
        let bindings = preparation
            .local_bindings()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        if bindings
            .iter()
            .any(|binding| environment.contains_key(binding))
            || bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len()
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let Some(code) = self.invocation_code(&preparation.expression, None)? else {
            return Ok(None);
        };
        let mut exports = Vec::with_capacity(bindings.len());
        for binding in bindings {
            self.tick(Some(source))?;
            let index = code
                .locals()
                .binary_search_by_key(&binding, |(id, _)| *id)
                .map_err(|_| self.failure(ErrorKind::MissingBinding, Some(source)))?;
            exports.push((binding, code.locals()[index].1));
        }
        let code = self.start_frame(&code, environment)?;
        self.bytecode_input_preparations += 1;
        Ok(Some(InputFrame {
            code,
            finish: InputFinish::Preparation { exports, source },
        }))
    }

    pub(super) fn evaluate_input_preparation(
        &mut self,
        preparation: &nir::program::NormalFormPreparation,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let Some(frame) = self.start_input_preparation(preparation, environment)? else {
            return self.eval(&preparation.expression, environment);
        };
        let result = frame.run(self);
        *environment = result.environment;
        result.flow
    }

    pub fn bytecode_input_preparations(&self) -> usize {
        self.bytecode_input_preparations
    }

    pub(super) fn start_frame(
        &mut self,
        code: &Arc<Program>,
        environment: &mut Environment,
    ) -> Result<Frame, Error> {
        self.effect_charge(
            code.registers(),
            code.instructions().first().map(|i| i.source),
        )?;
        let mut frame = Frame {
            code: code.clone(),
            environment: std::mem::take(environment),
            imported: Vec::new(),
            terminal: false,
            waiting: None,
            calls: BTreeMap::new(),
            normal_types: BTreeMap::new(),
            staged: None,
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

    pub(super) fn resume_frame(
        &mut self,
        frame: &mut Frame,
        quantum: usize,
    ) -> Result<Progress, Error> {
        if frame.terminal || frame.waiting.is_some() {
            return Err(self.failure(ErrorKind::InvalidControlFlow, None));
        }
        let result = self.run_frame(frame, quantum);
        // A failed instruction may have consumed operands or changed storage.
        // Such a frame is terminal; resumption must never replay that work.
        if matches!(result, Ok(Progress::Complete(_)) | Err(_)) {
            frame.terminal = true;
        }
        result
    }

    fn run_frame(&mut self, frame: &mut Frame, quantum: usize) -> Result<Progress, Error> {
        let code = frame.code.clone();
        for _ in 0..quantum {
            let instruction = code
                .instructions()
                .get(frame.pc)
                .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, None))?;
            let source = instruction.source;
            if matches!(&instruction.op, Op::Stage(operation) if !matches!(operation, comptime_compiler::staged_inputs::Instruction::Enter))
            {
                self.effect_charge(1, Some(source))?;
            } else {
                self.tick(Some(source))?;
            }
            frame.pc += 1;
            let missing = || Error {
                kind: ErrorKind::MissingBinding,
                source: Some(source),
            };
            match &instruction.op {
                Op::Stage(operation) => {
                    if let Some(flow) = frame
                        .staged
                        .get_or_insert_with(|| Box::new(staged_bytecode::State::default()))
                        .step(operation, &mut frame.slots, self, source)?
                    {
                        return Ok(Progress::Complete(flow));
                    }
                }
                Op::CheckListAppend { local, ty } => {
                    self.check_list_append(
                        frame.slots[*local].as_ref().ok_or_else(missing)?,
                        ty,
                        source,
                    )?;
                }
                Op::ListAppend { local, suffix, ty } => {
                    let suffix = frame.slots[*suffix].take().ok_or_else(missing)?;
                    self.complete_list_append(
                        frame.slots[*local].as_mut().ok_or_else(missing)?,
                        suffix,
                        ty,
                        source,
                    )?;
                }
                Op::PrepareListIndex(contract) => {
                    self.prepare_list_index(&contract.ty, &contract.list, &contract.index, source)?
                }
                Op::CheckIndexedList { value, contract } => {
                    self.index_list_length(
                        frame.slots[*value].as_ref().ok_or_else(missing)?,
                        &contract.list,
                        source,
                    )?;
                }
                Op::ListIndex {
                    destination,
                    list,
                    index,
                } => {
                    let list = frame.slots[*list].take().ok_or_else(missing)?;
                    let index = frame.slots[*index].take().ok_or_else(missing)?;
                    frame.slots[*destination] =
                        Some(self.complete_list_index(list, index, source)?);
                }
                Op::LocalListIndex {
                    destination,
                    contract,
                    list,
                    list_source,
                    index,
                } => {
                    use comptime_compiler::leaf_inputs::Selector;
                    use leaf_inputs::LocalSelector;
                    self.tick(Some(*list_source))?;
                    let list = frame.slots[*list].as_ref().ok_or_else(|| {
                        self.failure(ErrorKind::MissingBinding, Some(*list_source))
                    })?;
                    let index = match index {
                        Selector::Local(slot, at) => {
                            LocalSelector::Value(frame.slots[*slot].as_ref(), *at)
                        }
                        Selector::Literal(expr) => LocalSelector::Literal(expr),
                    };
                    let independent = matches!(
                        &index,
                        LocalSelector::Literal(_)
                            | LocalSelector::Value(Some(Value::Integer(_)), _)
                    );
                    let Flow::Value(value) = self.read_local_list_index(
                        list,
                        &contract.list,
                        independent,
                        *list_source,
                        source,
                        |this| {
                            this.read_leaf_selector(Some(index))
                                .map(|value| Flow::Value(value.unwrap()))
                        },
                    )?
                    else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::PrepareList(list) => {
                    self.prepare_list_construction(&list.ty, list.inputs.iter(), source)?;
                }
                Op::List {
                    destination,
                    list,
                    values,
                } => {
                    let values = values
                        .iter()
                        .map(|slot| frame.slots[*slot].take().ok_or_else(missing))
                        .collect::<Result<_, _>>()?;
                    frame.slots[*destination] =
                        Some(self.complete_list_construction(&list.ty, values, source)?);
                }
                Op::PrepareListOperation(operation) => self.prepare_list_operation(
                    &operation.ty,
                    &operation.left,
                    operation.right.as_ref(),
                    source,
                )?,
                Op::CheckListOperand { value, ty } => {
                    self.list_operand_length(
                        frame.slots[*value].as_ref().ok_or_else(missing)?,
                        ty,
                        source,
                    )?;
                }
                Op::ListLength {
                    destination,
                    value,
                    ty,
                    local,
                    value_source,
                } => {
                    let value = frame.slots[*value].as_ref().ok_or_else(|| {
                        self.failure(ErrorKind::MissingBinding, Some(*value_source))
                    })?;
                    let flow = if *local {
                        self.tick(Some(*value_source))?;
                        self.local_list_length(value, ty, *value_source, source)?
                    } else {
                        let length = self.list_value_length(value, source)?;
                        self.list_length_value(length, source)?
                    };
                    let Flow::Value(value) = flow else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::ListConcat {
                    destination,
                    left,
                    right,
                } => {
                    let left = frame.slots[*left].take().ok_or_else(missing)?;
                    let right = frame.slots[*right].take().ok_or_else(missing)?;
                    let length = self.list_value_length(&left, source)?;
                    let other_length = self.list_value_length(&right, source)?;
                    let Flow::Value(value) =
                        self.complete_list_concat(left, right, length, other_length, source)?
                    else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::LocalObjectQuery {
                    destination,
                    access,
                    receiver,
                    receiver_source,
                    selector,
                } => {
                    use comptime_compiler::leaf_inputs::Selector;
                    use comptime_compiler::objects::ObjectOperation;
                    use leaf_inputs::LocalSelector;
                    let ObjectOperation::Query(query) = access.operation else {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    };
                    let receiver = frame.slots[*receiver].as_ref().ok_or_else(|| {
                        self.failure(ErrorKind::MissingBinding, Some(*receiver_source))
                    })?;
                    let selector = selector.as_ref().map(|selector| match selector {
                        Selector::Local(slot, source) => {
                            LocalSelector::Value(frame.slots[*slot].as_ref(), *source)
                        }
                        Selector::Literal(expr) => LocalSelector::Literal(expr),
                    });
                    let Flow::Value(value) = self.query_local_object(
                        query,
                        &access.ty,
                        receiver,
                        *receiver_source,
                        selector,
                        source,
                    )?
                    else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::BeginContextualInput {
                    destination,
                    expected,
                    input,
                } => {
                    let mut state = super::normal_types::Inputs::contextual();
                    self.prepare_normal_shape(input, expected, &mut state.prepared, 0)?;
                    frame.normal_types.insert(*destination, state);
                }
                Op::BeginNormalType {
                    destination,
                    constructor,
                } => {
                    self.prepare_normal_type(
                        constructor.head,
                        &constructor.ty,
                        constructor.count,
                        constructor.diverges,
                        source,
                    )?;
                    frame.normal_types.insert(
                        *destination,
                        super::normal_types::Inputs::new(constructor.head, constructor.count),
                    );
                }
                Op::PrepareNormalInput {
                    destination,
                    index,
                    input,
                } => {
                    let state = frame
                        .normal_types
                        .get_mut(destination)
                        .ok_or_else(missing)?;
                    self.prepare_compiled_normal_input(state, *index, input, source)?;
                }
                Op::AppendNormalContainer {
                    constructor,
                    parent,
                    value,
                    spread,
                    diverges,
                } => {
                    if *diverges {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    let input = frame
                        .normal_types
                        .get_mut(constructor)
                        .ok_or_else(missing)?;
                    let parent = input.prepared.get_mut(parent).ok_or_else(missing)?;
                    self.append_compiled_normal_container(parent, value, *spread, source)?;
                }
                Op::FinishNormalContainer {
                    constructor,
                    value,
                    diverges,
                } => {
                    if *diverges {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let input = frame
                        .normal_types
                        .get_mut(constructor)
                        .ok_or_else(missing)?;
                    let container = input.prepared.remove(value).ok_or_else(missing)?;
                    frame.slots[*value] =
                        Some(self.finish_compiled_normal_container(container, source)?);
                }
                Op::NormalResult {
                    destination,
                    value,
                    diverges,
                } => {
                    if *diverges {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    frame.slots[*destination] = Some(value);
                }
                Op::NormalLiteral {
                    constructor,
                    destination,
                } => {
                    let input = frame
                        .normal_types
                        .get_mut(constructor)
                        .ok_or_else(missing)?;
                    let Some(super::normal_types::Prepared::Literal(value)) =
                        input.prepared.remove(destination)
                    else {
                        return Err(missing());
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::BindNormalInput {
                    destination,
                    value,
                    diverges,
                } => {
                    if *diverges {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    let input = frame
                        .normal_types
                        .get_mut(destination)
                        .ok_or_else(missing)?;
                    self.bind_compiled_normal_input(input, value, source)?;
                }
                Op::FinishNormalType { destination } => {
                    let input = frame
                        .normal_types
                        .remove(destination)
                        .and_then(|input| input.normal)
                        .ok_or_else(missing)?;
                    frame.slots[*destination] = Some(self.complete_normal_type(
                        input.head,
                        input.bindings.finish().ok_or_else(missing)?,
                        source,
                    )?);
                }
                Op::PrepareTypeConstructor(constructor) => {
                    let inputs: Vec<_> = constructor.inputs.iter().collect();
                    self.prepare_type_constructor(
                        &constructor.kind,
                        &constructor.ty,
                        &inputs,
                        source,
                    )?;
                }
                Op::CheckTypeConstructorInput {
                    constructor,
                    index,
                    value,
                } => {
                    let input = frame.slots[*value].take().ok_or_else(missing)?;
                    frame.slots[*value] = Some(Value::Type(self.check_type_constructor_input(
                        &constructor.kind,
                        *index,
                        constructor.inputs.len(),
                        input,
                        source,
                    )?));
                }
                Op::TypeConstructor {
                    destination,
                    constructor,
                    inputs,
                } => {
                    let types = inputs
                        .iter()
                        .map(|slot| match frame.slots[*slot].take() {
                            Some(Value::Type(ty)) => Ok(ty),
                            _ => Err(self.failure(ErrorKind::Type, Some(source))),
                        })
                        .collect::<Result<_, _>>()?;
                    frame.slots[*destination] =
                        Some(self.complete_type_constructor(&constructor.kind, types, source)?);
                }
                Op::PrepareTypeEqual { ty, inputs } => {
                    self.prepare_type_equal(ty, &inputs[0], &inputs[1], source)?
                }
                Op::TypeDescription(slot) => {
                    let value = frame.slots[*slot].take().ok_or_else(missing)?;
                    frame.slots[*slot] =
                        Some(Value::Type(self.type_description_term(value, source)?));
                }
                Op::TypeEqual {
                    destination,
                    left,
                    right,
                } => {
                    let Some(Value::Type(left)) = frame.slots[*left].take() else {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    };
                    let Some(Value::Type(right)) = frame.slots[*right].take() else {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    };
                    frame.slots[*destination] =
                        Some(self.complete_type_equal(left, right, source)?);
                }
                Op::PrepareObjectAccess(access) => {
                    use comptime_compiler::objects::ObjectOperation;
                    match access.operation {
                        ObjectOperation::Query(query) => {
                            self.prepare_object_query(query, &access.ty, source)?
                        }
                        ObjectOperation::Edit(edit) => {
                            self.prepare_object_edit(edit, &access.ty, &access.input, source)?
                        }
                    }
                }
                Op::ObjectAccess {
                    destination,
                    access,
                    packet,
                } => {
                    use comptime_compiler::objects::ObjectOperation;
                    let packet = frame.slots[*packet].take().ok_or_else(missing)?;
                    let result = match access.operation {
                        ObjectOperation::Query(query) => {
                            self.complete_object_query(query, &access.ty, packet, source)?
                        }
                        ObjectOperation::Edit(edit) => self.complete_object_edit(
                            edit,
                            &access.ty,
                            &access.input,
                            packet,
                            source,
                        )?,
                    };
                    let Flow::Value(value) = result else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    frame.slots[*destination] = Some(value);
                }
                Op::PrepareObject(object) => self.prepare_object(
                    &object.ty,
                    object.names.len(),
                    object.fields.len(),
                    object
                        .fields
                        .iter()
                        .any(|ty| *ty == Term::Primitive(Primitive::Never)),
                    source,
                )?,
                Op::CheckObjectEntry {
                    object,
                    index,
                    value,
                } => {
                    self.check_object_entry(
                        frame.slots[*value].as_ref().ok_or_else(missing)?,
                        &object.fields[*index],
                        object.names[*index].as_deref(),
                        source,
                    )?;
                }
                Op::Object {
                    destination,
                    object,
                    fields,
                } => {
                    let mut entries = Vec::with_capacity(fields.len());
                    for (index, slot) in fields.iter().enumerate() {
                        let value = ObjectValue {
                            ty: object.fields[index].clone(),
                            value: frame.slots[*slot].take().ok_or_else(missing)?,
                        };
                        entries.push(match &object.names[index] {
                            Some(key) => ObjectEntry::Property {
                                key: key.clone(),
                                value,
                            },
                            None => ObjectEntry::Value(value),
                        });
                    }
                    frame.slots[*destination] = Some(Value::Object(Object::from_entries(entries)));
                }
                Op::Tuple {
                    destination,
                    fields,
                } => {
                    let values = fields
                        .iter()
                        .map(|slot| frame.slots[*slot].take().ok_or_else(missing))
                        .collect::<Result<Vec<_>, _>>()?;
                    frame.slots[*destination] = Some(Value::Tuple(values));
                }

                Op::MatchPattern {
                    destination,
                    value,
                    pattern,
                    bindings,
                } => {
                    // Only structural patterns are admitted by the compiler.
                    // Keep candidate bindings private until the complete pattern succeeds.
                    let mut candidate = Environment::new();
                    for (id, slot) in bindings {
                        if let Some(value) = &frame.slots[*slot] {
                            candidate.insert(*id, self.checkpoint_representation(value, source)?);
                        }
                    }
                    let subject = frame.slots[*value].as_ref().ok_or_else(missing)?;
                    let (matched, _) =
                        self.match_pattern_with_bindings(pattern, subject, &mut candidate)?;
                    let Flow::Value(Value::Bool(matched)) = matched else {
                        return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                    };
                    if matched {
                        for (id, slot) in bindings {
                            if let Some(value) = candidate.remove(id) {
                                if frame.slots[*slot].is_none() {
                                    frame.scopes.last_mut().ok_or_else(missing)?.push(*slot);
                                }
                                frame.slots[*slot] = Some(value);
                            } else {
                                frame.slots[*slot] = None;
                            }
                        }
                    }
                    frame.slots[*destination] = Some(Value::Bool(matched));
                }
                Op::CommitScope => {
                    let bindings = frame.scopes.pop().ok_or_else(missing)?;
                    if let Some(parent) = frame.scopes.last_mut() {
                        parent.extend(bindings);
                    }
                }
                Op::InvalidControlFlow => {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                }
                Op::NonExhaustiveMatch => {
                    return Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(source)));
                }

                Op::PrepareOptional { ty, input } => {
                    self.prepare_optional(
                        ty,
                        input.as_ref().map(|(ty, at)| (ty.as_ref(), *at)),
                        source,
                    )?;
                }
                Op::Optional {
                    destination,
                    ty,
                    value,
                    value_source,
                } => {
                    let value = value
                        .map(|slot| frame.slots[slot].take().ok_or_else(missing))
                        .transpose()?;
                    frame.slots[*destination] =
                        Some(self.complete_optional(ty, value, *value_source)?);
                }
                Op::PrepareError(conversion) => {
                    self.prepare_error_conversion(
                        conversion.conversion,
                        &conversion.input,
                        &conversion.output,
                        source,
                    )?;
                }
                Op::ErrorConvert {
                    destination,
                    value,
                    conversion,
                } => {
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    if !self.has_type(&value, &conversion.input) {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    frame.slots[*destination] = Some(self.complete_error_conversion(
                        conversion.conversion,
                        &conversion.input,
                        &conversion.output,
                        value,
                        source,
                    )?);
                }

                Op::Variant {
                    destination,
                    declaration,
                    index,
                    ty,
                    fields,
                } => {
                    let values = fields
                        .iter()
                        .map(|field| frame.slots[*field].take().ok_or_else(missing))
                        .collect::<Result<Vec<_>, _>>()?;
                    frame.slots[*destination] =
                        Some(self.construct_variant(ty, *declaration, *index, values, source)?);
                }

                Op::Record {
                    destination,
                    declaration,
                    ty,
                    fields,
                } => {
                    let values = fields
                        .iter()
                        .map(|field| frame.slots[*field].take().ok_or_else(missing))
                        .collect::<Result<Vec<_>, _>>()?;
                    frame.slots[*destination] =
                        Some(self.construct_record(ty, *declaration, values, source)?);
                }

                Op::CheckBinding(local) => {
                    frame.slots[*local].as_ref().ok_or_else(missing)?;
                }
                Op::WritePlace {
                    local,
                    value,
                    fields,
                } => {
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    let target = frame.slots[*local].as_mut().ok_or_else(missing)?;
                    self.write_comptime_place(target, fields, value, source)?;
                }
                Op::Field {
                    destination,
                    base,
                    local,
                    nominal,
                    name,
                    index,
                    ty,
                    base_source,
                } => {
                    let temporary = if *local {
                        None
                    } else {
                        frame.slots[*base].take()
                    };
                    let record = if *local {
                        frame.slots[*base].as_ref()
                    } else {
                        temporary.as_ref()
                    }
                    .ok_or_else(missing)?;
                    let value = self.read_record_field(
                        record,
                        *nominal,
                        name.as_deref(),
                        *index,
                        ty,
                        *base_source,
                        source,
                    )?;
                    frame.slots[*destination] = Some(value);
                }

                Op::BeginCall {
                    call,
                    callee,
                    parameter_bindings,
                    prepare_defaults,
                } => {
                    let callee = frame.slots[*callee].take().ok_or_else(missing)?;
                    let parameters = self.call_parameters(&callee, source)?;
                    if !parameter_bindings.is_empty() && *parameter_bindings != parameters {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                    }
                    self.effect_charge(parameters.len(), Some(source))?;
                    frame.calls.insert(
                        *call,
                        PendingCall {
                            callee,
                            arguments: (0..parameters.len()).map(|_| None).collect(),
                            parameter_bindings: if *prepare_defaults {
                                parameter_bindings.clone()
                            } else {
                                vec![]
                            },
                            defaults: Environment::new(),
                            last_default: None,
                        },
                    );
                }
                Op::CheckCallArgument {
                    call,
                    parameter,
                    is_default,
                    passing,
                    argument_source,
                } => {
                    if *passing != nir::program::ArgumentPassing::Value
                        && (*is_default || *parameter != 0)
                    {
                        return Err(
                            self.failure(ErrorKind::InvalidCallPlan, Some(*argument_source))
                        );
                    }
                    let pending = frame.calls.get_mut(call).ok_or_else(missing)?;
                    if *parameter >= pending.arguments.len()
                        || pending.arguments[*parameter].is_some()
                    {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                    }
                    if *is_default {
                        if pending.parameter_bindings.is_empty()
                            || pending.last_default.is_some_and(|last| last >= *parameter)
                        {
                            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                        }
                        pending.last_default = Some(*parameter);
                    } else if pending.last_default.is_some() {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                    }
                }
                Op::Reference {
                    destination,
                    base,
                    local,
                    fields,
                    rebind,
                } => {
                    let reference = if *local {
                        value_reference::ValueReference::for_slot(
                            frame.slots[*base].as_mut().ok_or_else(missing)?,
                        )
                    } else {
                        value_reference::ValueReference::root(Storage::new(
                            frame.slots[*base].take().ok_or_else(missing)?,
                        ))
                    };
                    let mut reference = self.project_value_reference(reference, fields, source)?;
                    if !rebind {
                        reference.prohibit_rebind();
                    }
                    frame.slots[*destination] = Some(Value::ComptimeReference(reference));
                }
                Op::DefaultArgument {
                    call,
                    destination,
                    value,
                    dynamic,
                } => {
                    let pending = frame.calls.get_mut(call).ok_or_else(missing)?;
                    let expression = if *dynamic
                        && matches!(&pending.callee,
                        Value::Closure(closure) if closure.declaration_instance().is_some())
                    {
                        Arc::new(self.instance_default_expression(
                            source,
                            value,
                            &pending.callee,
                        )?)
                    } else {
                        value.clone()
                    };
                    let environment = std::mem::take(&mut pending.defaults);
                    frame.waiting = Some(Waiting::Default {
                        call: *call,
                        destination: *destination,
                    });
                    return Ok(Progress::Default {
                        expression,
                        environment,
                    });
                }
                Op::StoreCallArgument {
                    call,
                    parameter,
                    value,
                } => {
                    let value = frame.slots[*value].take().ok_or_else(missing)?;
                    let pending = frame.calls.get_mut(call).ok_or_else(missing)?;
                    let slot = pending.arguments.get_mut(*parameter).ok_or_else(missing)?;
                    let value = self.prepare_argument(*parameter, source, value)?;
                    if !pending.parameter_bindings.is_empty() {
                        pending
                            .defaults
                            .insert(pending.parameter_bindings[*parameter], value.clone());
                    }
                    *slot = Some(value);
                }
                Op::Invoke { call, destination } => {
                    let pending = frame.calls.remove(call).ok_or_else(missing)?;
                    let arguments = pending
                        .arguments
                        .into_iter()
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| self.failure(ErrorKind::Arity, Some(source)))?;
                    frame.waiting = Some(Waiting::Call(*destination));
                    return Ok(Progress::Call {
                        callee: pending.callee,
                        arguments,
                    });
                }
                Op::PrepareIntrinsic {
                    tag,
                    binding,
                    inputs,
                    output,
                } => {
                    let binding = binding
                        .as_deref()
                        .filter(|binding| binding.is_comptime())
                        .ok_or_else(|| self.failure(ErrorKind::Unsupported, Some(source)))?;
                    binding
                        .validate_types(*tag, inputs.iter(), output)
                        .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                }
                Op::Intrinsic {
                    destination,
                    binding,
                    inputs,
                } => {
                    let binding = binding
                        .as_deref()
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
                        K::Function(_) => self.eval_leaf(value, &mut frame.environment)?,
                        K::FunctionInstance { .. } => {
                            self.eval_function_instance(value, &mut frame.environment)?
                        }
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
                    self.prepare_scalar(scalar.descriptor(), source)?;
                }
                Op::CheckScalarOperand { scalar, value } => {
                    let plan = self.prepare_scalar(scalar.descriptor(), source)?;
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
                    let plan = self.prepare_scalar(scalar.descriptor(), source)?;
                    let a = frame.slots[*left].take().ok_or_else(missing)?;
                    let b = right
                        .map(|slot| frame.slots[slot].take().ok_or_else(missing))
                        .transpose()?;
                    frame.slots[*destination] =
                        Some(self.scalar_values(scalar.descriptor(), plan, a, b, source)?);
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
                        self.write_comptime_place(target, &[], value, source)?;
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
                            if *ty == Term::Primitive(Primitive::BitVec(value.width())) =>
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
    fn object_access_bytecode_preserves_domains_errors_and_recovery() {
        use nir::program::{ObjectEditKind as O, ObjectQueryKind as Q};
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let boolean = Term::Primitive(Primitive::Bool);
        let object_ty = Term::Primitive(Primitive::Object);
        let integer = Term::Primitive(Primitive::Integer);
        let symbol = Term::Primitive(Primitive::Symbol);
        let local = |index, ty| Expr {
            source,
            ty,
            kind: K::Local(LocalId { owner: 1, index }),
        };
        let original = Value::Object(Object::from_entries(vec![
            ObjectEntry::Property {
                key: "flag".into(),
                value: ObjectValue {
                    ty: boolean.clone(),
                    value: Value::Bool(true),
                },
            },
            ObjectEntry::Property {
                key: "flag".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Char),
                    value: Value::Char('x'),
                },
            },
        ]));
        let mut bodies = Vec::new();
        for query in [Q::Length, Q::Key, Q::EntryType, Q::Value, Q::NamedValue] {
            let mut fields = vec![local(0, object_ty.clone())];
            if query != Q::Length {
                fields.push(local(
                    1,
                    if query == Q::NamedValue {
                        symbol.clone()
                    } else {
                        integer.clone()
                    },
                ));
            }
            let ty = match query {
                Q::Length => integer.clone(),
                Q::Key => Term::Optional(Box::new(symbol.clone())),
                Q::EntryType => Term::Primitive(Primitive::Type),
                _ => Term::Optional(Box::new(boolean.clone())),
            };
            let subject = Box::new(Expr {
                source,
                ty: Term::Tuple(fields.iter().map(|v| v.ty.clone()).collect()),
                kind: K::Tuple(fields),
            });
            bodies.push(Expr {
                source,
                ty,
                kind: K::ObjectQuery { query, subject },
            });
        }
        for operation in [O::Concat, O::Append, O::Replace, O::Remove] {
            let mut fields = vec![
                local(0, object_ty.clone()),
                local(
                    1,
                    match operation {
                        O::Concat => object_ty.clone(),
                        O::Append => Term::Optional(Box::new(symbol.clone())),
                        _ => integer.clone(),
                    },
                ),
            ];
            if matches!(operation, O::Append | O::Replace) {
                fields.push(local(2, boolean.clone()));
            }
            let subject = Box::new(Expr {
                source,
                ty: Term::Tuple(fields.iter().map(|v| v.ty.clone()).collect()),
                kind: K::Tuple(fields),
            });
            bodies.push(Expr {
                source,
                ty: object_ty.clone(),
                kind: K::ObjectEdit { operation, subject },
            });
        }
        for body in bodies {
            let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
            let instruction = if matches!(body.kind, K::ObjectQuery { .. }) {
                "(local-object-query "
            } else {
                "(object-access "
            };
            assert!(code.to_string().contains(instruction));
            let mut tree = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            // Invalid payloads, bounds, keys, and absence followed by valid inputs
            // exercise recovery with the same interpreter and compiled program.
            for selector in [
                Value::Char('!'),
                Value::Integer(comptime_runtime::Integer::from_u128(99)),
                Value::Symbol("missing".into()),
                Value::Symbol("flag".into()),
                original.clone(),
                Value::Optional {
                    ty: Term::Optional(Box::new(symbol.clone())),
                    value: None,
                },
                Value::Integer(comptime_runtime::Integer::from_u128(0)),
                Value::Integer(comptime_runtime::Integer::from_u128(1)),
            ] {
                for payload in [Value::Char('!'), Value::Bool(false)] {
                    let mut environment = Environment::new();
                    environment.insert(LocalId { owner: 1, index: 0 }, original.clone());
                    environment.insert(LocalId { owner: 1, index: 1 }, selector.clone());
                    environment.insert(LocalId { owner: 1, index: 2 }, payload);
                    let mut expected_environment = environment.clone();
                    let value = |result: Result<Flow, Error>| {
                        result.map(|flow| match flow {
                            Flow::Value(value) => value,
                            _ => panic!("unexpected flow"),
                        })
                    };
                    assert_eq!(
                        value(vm.execute_code(&code, &mut environment)),
                        value(tree.eval(&body, &mut expected_environment)),
                        "{body:?}"
                    );
                    assert_eq!(
                        environment.get(&LocalId { owner: 1, index: 0 }),
                        Some(&original)
                    );
                }
            }
            let mut invalid = body.clone();
            invalid.ty = boolean.clone();
            let code = Arc::new(comptime_compiler::compile(&invalid, &mut 10000).unwrap());
            assert_eq!(
                vm.execute_code(&code, &mut Environment::new())
                    .unwrap_err()
                    .kind,
                ErrorKind::Type
            );
            vm.comptime_only = false;
            assert_eq!(
                vm.execute_code(&code, &mut Environment::new())
                    .unwrap_err()
                    .kind,
                ErrorKind::PhaseMismatch
            );
        }
    }

    #[test]
    fn object_validation_orders_effects_and_stops_before_later_entries() {
        let package = Package::default();
        let source = SourceRange { start: 7, end: 9 };
        let trace = LocalId { owner: 1, index: 0 };
        let boolean = Term::Primitive(Primitive::Bool);
        let field = |effect, malformed| Expr {
            source,
            ty: boolean.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::Assign {
                        place: nir::program::Place {
                            root: trace,
                            fields: vec![],
                        },
                        value: Box::new(Expr {
                            source,
                            ty: boolean.clone(),
                            kind: K::Literal(Literal::Bool(effect)),
                        }),
                    },
                }],
                result: Some(Box::new(Expr {
                    source,
                    ty: boolean.clone(),
                    kind: if malformed {
                        K::Unit
                    } else {
                        K::Literal(Literal::Bool(true))
                    },
                })),
            },
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for case in 0..3 {
                let body = Expr {
                    source,
                    ty: Term::Primitive(Primitive::Object),
                    kind: K::Object {
                        names: if case == 0 {
                            vec![]
                        } else {
                            vec![None, Some("same".into())]
                        },
                        fields: vec![field(true, case == 1), field(false, false)],
                    },
                };
                let mut environment = Environment::new();
                environment.insert(trace, Value::Bool(false));
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
                    vm.execute_code(&code, &mut environment)
                } else {
                    vm.eval(&body, &mut environment)
                };
                assert_eq!(environment.get(&trace), Some(&Value::Bool(case == 1)));
                if case < 2 {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                } else {
                    assert!(matches!(result.unwrap(), Flow::Value(Value::Object(_))));
                }
            }
        }
    }

    #[test]
    fn failed_declarations_reject_missing_or_fallthrough_else_without_publishing() {
        use nir::program::Pattern;
        let package = Package::default();
        let source = SourceRange { start: 7, end: 9 };
        let binding = LocalId { owner: 1, index: 0 };
        let unit = Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Unit,
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            for (otherwise, expected) in [
                (None, ErrorKind::NonExhaustiveMatch),
                (Some(Box::new(unit.clone())), ErrorKind::InvalidControlFlow),
            ] {
                let body = Expr {
                    source,
                    ty: unit.ty.clone(),
                    kind: K::LetPattern {
                        pattern: Pattern::Binding {
                            binding,
                            inner: Box::new(Pattern::Wild),
                        },
                        value: Box::new(unit.clone()),
                        guard: Some(Box::new(Expr {
                            source,
                            ty: Term::Primitive(Primitive::Bool),
                            kind: K::Literal(Literal::Bool(false)),
                        })),
                        otherwise,
                    },
                };
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
                    vm.execute_code(&code, &mut environment)
                } else {
                    vm.eval(&body, &mut environment)
                };
                assert_eq!(result.unwrap_err().kind, expected);
                assert!(!environment.contains_key(&binding));
            }
        }
    }

    #[test]
    fn compiled_match_rejects_missing_arms_and_bad_guards_then_recovers() {
        use nir::program::{MatchArm, Pattern};
        let package = Package::default();
        let source = SourceRange { start: 7, end: 9 };
        let boolean = Term::Primitive(Primitive::Bool);
        let literal = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(Literal::Bool(true)),
        };
        let binding = LocalId { owner: 1, index: 0 };
        let mut body = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Match {
                value: Box::new(literal.clone()),
                arms: vec![],
            },
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            let run = |vm: &mut Interpreter<'_>, body: &Expr, environment: &mut Environment| {
                if compiled {
                    let code = Arc::new(comptime_compiler::compile(body, &mut 1000).unwrap());
                    vm.execute_code(&code, environment)
                } else {
                    vm.eval(body, environment)
                }
            };
            assert!(matches!(
                run(&mut vm, &body, &mut environment),
                Err(Error {
                    kind: ErrorKind::NonExhaustiveMatch,
                    ..
                })
            ));
            let K::Match { arms, .. } = &mut body.kind else {
                unreachable!()
            };
            arms.push(MatchArm {
                pattern: Pattern::Binding {
                    binding,
                    inner: Box::new(Pattern::Wild),
                },
                guard: Some(Expr {
                    source,
                    ty: boolean.clone(),
                    kind: K::Unit,
                }),
                body: Expr {
                    source,
                    ty: boolean.clone(),
                    kind: K::Local(binding),
                },
            });
            assert!(matches!(run(&mut vm, &body, &mut environment),
                Err(Error { kind: ErrorKind::Type, source: Some(at) }) if at == source));
            assert!(!environment.contains_key(&binding));
            let K::Match { arms, .. } = &mut body.kind else {
                unreachable!()
            };
            arms[0].guard = None;
            assert!(matches!(
                run(&mut vm, &body, &mut environment).unwrap(),
                Flow::Value(Value::Bool(true))
            ));
            assert!(!environment.contains_key(&binding));
            let K::Match { arms, .. } = &mut body.kind else {
                unreachable!()
            };
            arms.clear();
        }
    }

    #[test]
    fn qualified_construction_checks_plans_before_effects_and_payloads_afterward() {
        use nir::error_results::Conversion;
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let trace = LocalId { owner: 1, index: 0 };
        let boolean = Term::Primitive(Primitive::Bool);
        let optional = Term::Optional(Box::new(boolean.clone()));
        let error = Term::ErrorResult {
            errors: vec![Term::Primitive(Primitive::Char)],
            result: Box::new(boolean.clone()),
        };
        let operand = |kind| Expr {
            source,
            ty: boolean.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::Assign {
                        place: nir::program::Place {
                            root: trace,
                            fields: vec![],
                        },
                        value: Box::new(Expr {
                            source,
                            ty: boolean.clone(),
                            kind: K::Literal(Literal::Bool(true)),
                        }),
                    },
                }],
                result: Some(Box::new(Expr {
                    source,
                    ty: boolean.clone(),
                    kind,
                })),
            },
        };
        let present = |ty, operand| Expr {
            source,
            ty,
            kind: K::OptionalPresent(Box::new(operand)),
        };
        let converted = |ty, conversion, operand| Expr {
            source,
            ty,
            kind: K::ErrorConvert {
                conversion,
                value: Box::new(operand),
            },
        };
        for (body, effect) in [
            (
                present(
                    Term::Optional(Box::new(Term::Primitive(Primitive::Char))),
                    operand(K::Literal(Literal::Bool(true))),
                ),
                false,
            ),
            (present(optional.clone(), operand(K::Unit)), true),
            (
                converted(
                    optional,
                    Conversion::Success,
                    operand(K::Literal(Literal::Bool(true))),
                ),
                false,
            ),
            (
                converted(
                    error.clone(),
                    Conversion::Failure,
                    operand(K::Literal(Literal::Bool(true))),
                ),
                false,
            ),
            (
                converted(error, Conversion::Success, operand(K::Unit)),
                true,
            ),
        ] {
            let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
            for compiled in [false, true] {
                let mut environment = Environment::new();
                environment.insert(trace, Value::Bool(false));
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let result = if compiled {
                    vm.execute_code(&code, &mut environment)
                } else {
                    vm.eval(&body, &mut environment)
                };
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                assert_eq!(environment.get(&trace), Some(&Value::Bool(effect)));
            }
        }
    }

    #[test]
    fn product_error_transport_preserves_labels_and_payloads_inside_records() {
        use nir::error_results::Conversion;
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let boolean = Term::Primitive(Primitive::Bool);
        let character = Term::Primitive(Primitive::Char);
        let original = Term::ErrorResult {
            errors: vec![character.clone(), boolean.clone()],
            result: Box::new(boolean.clone()),
        };
        let reordered = Term::ErrorResult {
            errors: vec![boolean, character.clone()],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let input = Term::Record(vec![("entry".into(), original.clone())]);
        let output = Term::Record(vec![("entry".into(), reordered.clone())]);
        let body = Expr {
            source,
            ty: output,
            kind: K::ErrorConvert {
                conversion: Conversion::Product,
                value: Box::new(Expr {
                    source,
                    ty: input,
                    kind: K::AnonymousRecord {
                        fields: vec![Expr {
                            source,
                            ty: original,
                            kind: K::ErrorConvert {
                                conversion: Conversion::Failure,
                                value: Box::new(Expr {
                                    source,
                                    ty: character,
                                    kind: K::Literal(Literal::Char('x')),
                                }),
                            },
                        }],
                    },
                }),
            },
        };
        let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
        assert!(code.to_string().contains("(error-conversion product"));
        let expected = Value::AnonymousRecord(vec![(
            "entry".into(),
            Value::ErrorResult {
                ty: reordered,
                error: Some(1),
                value: Box::new(Value::Char('x')),
            },
        )]);
        // Product transport preserves the field types; it is not covariance.
        let mut widening = body.clone();
        if let K::ErrorConvert { value, .. } = &mut widening.kind {
            let Term::Record(fields) = &mut value.ty else {
                unreachable!()
            };
            let Term::ErrorResult { errors, .. } = &mut fields[0].1 else {
                unreachable!()
            };
            errors.pop();
            let narrowed = fields[0].1.clone();
            let K::AnonymousRecord { fields } = &mut value.kind else {
                unreachable!()
            };
            fields[0].ty = narrowed;
        }
        let widening_code = Arc::new(comptime_compiler::compile(&widening, &mut 1000).unwrap());
        for compiled in [false, true] {
            let mut environment = Environment::new();
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let K::ErrorConvert { value, .. } = &widening.kind else {
                unreachable!()
            };
            assert!(matches!(
                vm.eval(value, &mut environment).unwrap(),
                Flow::Value(_)
            ));
            let rejected = if compiled {
                vm.execute_code(&widening_code, &mut environment)
            } else {
                vm.eval(&widening, &mut environment)
            };
            assert!(matches!(
                rejected,
                Err(Error {
                    kind: ErrorKind::Type,
                    ..
                })
            ));
            let result = if compiled {
                vm.execute_code(&code, &mut environment)
            } else {
                vm.eval(&body, &mut environment)
            };
            assert!(matches!(result.unwrap(), Flow::Value(value) if value == expected));
        }
    }

    #[test]
    fn record_validation_follows_effects_but_early_exit_skips_remaining_fields() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let trace = LocalId { owner: 1, index: 0 };
        let boolean = Term::Primitive(Primitive::Bool);
        let literal = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(Literal::Bool(true)),
        };
        let field = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::Assign {
                        place: nir::program::Place {
                            root: trace,
                            fields: vec![],
                        },
                        value: Box::new(literal.clone()),
                    },
                }],
                result: Some(Box::new(literal.clone())),
            },
        };
        let invalid = Expr {
            source,
            ty: Term::Record(vec![("flag".into(), Term::Primitive(Primitive::Str))]),
            kind: K::AnonymousRecord {
                fields: vec![field.clone()],
            },
        };
        let early = Expr {
            source,
            ty: Term::Record(vec![
                ("first".into(), boolean.clone()),
                ("second".into(), boolean),
            ]),
            kind: K::AnonymousRecord {
                fields: vec![
                    Expr {
                        source,
                        ty: Term::Primitive(Primitive::Never),
                        kind: K::Return(Some(Box::new(literal))),
                    },
                    field,
                ],
            },
        };
        for (body, exits) in [(invalid, false), (early, true)] {
            let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
            assert!(code.to_string().contains("(record "));
            for compiled in [false, true] {
                let mut environment = Environment::new();
                environment.insert(trace, Value::Bool(false));
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let result = if compiled {
                    vm.execute_code(&code, &mut environment)
                } else {
                    vm.eval(&body, &mut environment)
                };
                if exits {
                    assert!(matches!(result.unwrap(), Flow::Return(Value::Bool(true))));
                    assert_eq!(environment.get(&trace), Some(&Value::Bool(false)));
                } else {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                    assert_eq!(environment.get(&trace), Some(&Value::Bool(true)));
                }
            }
        }
    }

    #[test]
    fn anonymous_field_reads_validate_name_index_and_result_type() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let root = LocalId { owner: 1, index: 0 };
        let boolean = Term::Primitive(Primitive::Bool);
        for (name, index, ty, valid) in [
            ("flag", 0, boolean.clone(), true),
            ("other", 0, boolean.clone(), false),
            ("flag", 1, boolean.clone(), false),
            ("flag", 0, Term::Primitive(Primitive::U8), false),
        ] {
            let body = Expr {
                source,
                ty,
                kind: K::AnonymousField {
                    value: Box::new(Expr {
                        source,
                        ty: Term::Record(vec![("flag".into(), boolean.clone())]),
                        kind: K::Local(root),
                    }),
                    name: name.into(),
                    index,
                },
            };
            let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
            assert!(code.to_string().contains("(field "));
            let stored = Value::AnonymousRecord(vec![("flag".into(), Value::Bool(true))]);
            let mut environment = Environment::new();
            environment.insert(root, stored.clone());
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let result = vm.execute_code(&code, &mut environment);
            if valid {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Bool(true))));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
            assert_eq!(environment.get(&root), Some(&stored));
        }
    }

    #[test]
    fn missing_assignment_root_prevents_rhs_effects() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let trace = LocalId { owner: 1, index: 0 };
        let missing = LocalId { owner: 1, index: 1 };
        let boolean = Term::Primitive(Primitive::Bool);
        let void = Term::Primitive(Primitive::Void);
        let literal = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(Literal::Bool(true)),
        };
        let write = |root, value| Expr {
            source,
            ty: void.clone(),
            kind: K::Assign {
                place: nir::program::Place {
                    root,
                    fields: vec![],
                },
                value: Box::new(value),
            },
        };
        let body = write(
            missing,
            Expr {
                source,
                ty: boolean,
                kind: K::Block {
                    statements: vec![write(trace, literal.clone())],
                    result: Some(Box::new(literal)),
                },
            },
        );
        let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
        let mut environment = Environment::new();
        environment.insert(trace, Value::Bool(false));
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.execute_code(&code, &mut environment).unwrap_err().kind,
            ErrorKind::MissingBinding
        );
        assert_eq!(environment.get(&trace), Some(&Value::Bool(false)));
        assert_eq!(
            vm.eval(&body, &mut environment).unwrap_err().kind,
            ErrorKind::MissingBinding
        );
        assert_eq!(environment.get(&trace), Some(&Value::Bool(false)));
    }

    #[test]
    fn receiver_path_is_revalidated_after_later_arguments_change_storage() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let boolean = Term::Primitive(Primitive::Bool);
        let root = LocalId { owner: 1, index: 0 };
        let function = LocalId { owner: 1, index: 1 };
        let parameters = vec![
            LocalId { owner: 2, index: 0 },
            LocalId { owner: 2, index: 1 },
        ];
        let signature = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: parameters
                .iter()
                .map(|_| nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: boolean.clone(),
                })
                .collect(),
            result: Box::new(boolean.clone()),
        }));
        let literal = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(Literal::Bool(true)),
        };
        let closure = Value::Closure(Arc::new(Closure {
            definition_source: None,
            origin: None,
            instance_arguments: None,
            parameters,
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(literal.clone()),
            ty: signature.clone(),
        }));
        let receiver = Expr {
            source,
            ty: boolean.clone(),
            kind: K::AnonymousField {
                value: Box::new(Expr {
                    source,
                    ty: Term::Record(vec![("flag".into(), boolean.clone())]),
                    kind: K::Local(root),
                }),
                name: "flag".into(),
                index: 0,
            },
        };
        // Deliberately malformed typed IR invalidates the acquired path after
        // selection. Storage updates must survive failure, but entry must fail.
        let later = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::Assign {
                        place: nir::program::Place {
                            root,
                            fields: vec![],
                        },
                        value: Box::new(Expr {
                            source,
                            ty: Term::Primitive(Primitive::Void),
                            kind: K::Unit,
                        }),
                    },
                }],
                result: Some(Box::new(literal)),
            },
        };
        let body = Expr {
            source,
            ty: boolean,
            kind: K::Call {
                callee: Box::new(Expr {
                    source,
                    ty: signature,
                    kind: K::Local(function),
                }),
                arguments: vec![
                    nir::program::CallArgument {
                        passing: nir::program::ArgumentPassing::ComptimeReference,
                        parameter: 0,
                        value: receiver,
                        is_default: false,
                    },
                    nir::program::CallArgument {
                        passing: nir::program::ArgumentPassing::Value,
                        parameter: 1,
                        value: later,
                        is_default: false,
                    },
                ],
                parameter_bindings: vec![],
            },
        };
        let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
        let mut environment = Environment::new();
        environment.insert(function, closure);
        environment.insert(
            root,
            Value::AnonymousRecord(vec![("flag".into(), Value::Bool(false))]),
        );
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.execute_code(&code, &mut environment).unwrap_err().kind,
            ErrorKind::InvalidValueReference
        );
        assert_eq!(
            vm.local_value(&environment, root, Some(source)).unwrap(),
            Value::Unit
        );
        assert_eq!(vm.depth, 0);
    }

    #[test]
    fn invalid_call_preparation_is_checked_before_evaluating_the_next_expression() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let boolean = Term::Primitive(Primitive::Bool);
        let binding = LocalId { owner: 1, index: 0 };
        let parameters = vec![
            LocalId { owner: 2, index: 0 },
            LocalId { owner: 2, index: 1 },
        ];
        let signature = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: parameters
                .iter()
                .map(|_| nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: boolean.clone(),
                })
                .collect(),
            result: Box::new(boolean.clone()),
        }));
        let literal = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Literal(Literal::Bool(true)),
        };
        let closure = Value::Closure(Arc::new(Closure {
            definition_source: None,
            origin: None,
            instance_arguments: None,
            parameters: parameters.clone(),
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(literal.clone()),
            ty: signature.clone(),
        }));
        for fault in 0..5 {
            let mut arguments: Vec<_> = (0..2)
                .map(|parameter| nir::program::CallArgument {
                    passing: nir::program::ArgumentPassing::Value,
                    parameter,
                    is_default: true,
                    value: literal.clone(),
                })
                .collect();
            let mut parameter_bindings = parameters.clone();
            match fault {
                0 => arguments.reverse(),
                1 => arguments[1].is_default = false,
                2 => parameter_bindings.clear(),
                3 => {
                    arguments[0].is_default = false;
                    arguments[1].is_default = false;
                    arguments[1].passing = nir::program::ArgumentPassing::ComptimeReference;
                }
                4 => arguments[0].passing = nir::program::ArgumentPassing::ComptimeReference,
                _ => unreachable!(),
            }
            // This expression would fail with MissingBinding if evaluated.
            arguments[1].value.kind = K::Local(LocalId {
                owner: 99,
                index: 0,
            });
            let body = Expr {
                source,
                ty: boolean.clone(),
                kind: K::Call {
                    callee: Box::new(Expr {
                        source,
                        ty: signature.clone(),
                        kind: K::Local(binding),
                    }),
                    arguments,
                    parameter_bindings,
                },
            };
            let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
            let mut environment = Environment::new();
            environment.insert(binding, closure.clone());
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.execute_code(&code, &mut environment).unwrap_err().kind,
                ErrorKind::InvalidCallPlan
            );
        }
    }

    #[test]
    fn pending_call_requires_delivery_and_rejects_bad_slots_before_effects() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let function = LocalId { owner: 1, index: 0 };
        let trace = LocalId { owner: 1, index: 1 };
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(boolean.clone()),
        }));
        let closure = Value::Closure(Arc::new(Closure {
            definition_source: None,
            origin: None,
            instance_arguments: None,
            parameters: vec![],
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(Expr {
                source,
                ty: boolean.clone(),
                kind: K::Literal(Literal::Bool(true)),
            }),
            ty: signature.clone(),
        }));
        let body = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Call {
                callee: Box::new(Expr {
                    source,
                    ty: signature,
                    kind: K::Local(function),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        environment.insert(function, closure);
        environment.insert(trace, Value::Bool(true));
        let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
        let mut frame = vm.start_frame(&code, &mut environment).unwrap();
        let Progress::Call { callee, arguments } = vm.resume_frame(&mut frame, 100).unwrap() else {
            panic!("expected suspended call");
        };
        let stopped = frame.to_string();
        assert_eq!(
            vm.resume_frame(&mut frame, 100).unwrap_err().kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(frame.to_string(), stopped);
        let result = vm.invoke_value_scoped(callee, arguments, None).unwrap();
        frame.deliver(result).unwrap();
        assert_eq!(
            frame.deliver(Value::Bool(false)).unwrap_err().kind,
            ErrorKind::InvalidControlFlow
        );
        assert!(matches!(
            vm.resume_frame(&mut frame, 100).unwrap(),
            Progress::Complete(Flow::Value(Value::Bool(true)))
        ));
        environment = frame.restore_environment();

        let mut invalid = body;
        let K::Call { arguments, .. } = &mut invalid.kind else {
            unreachable!()
        };
        arguments.push(nir::program::CallArgument {
            passing: nir::program::ArgumentPassing::Value,
            parameter: 0,
            is_default: false,
            value: Expr {
                source,
                ty: Term::Primitive(Primitive::Void),
                kind: K::Assign {
                    place: nir::program::Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(Expr {
                        source,
                        ty: boolean,
                        kind: K::Literal(Literal::Bool(false)),
                    }),
                },
            },
        });
        let code = Arc::new(comptime_compiler::compile(&invalid, &mut 1000).unwrap());
        assert_eq!(
            vm.execute_code(&code, &mut environment).unwrap_err().kind,
            ErrorKind::InvalidCallPlan
        );
        assert_eq!(environment.get(&trace), Some(&Value::Bool(true)));
    }

    #[test]
    fn suspended_frame_owns_code_after_source_and_cache_are_released() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        let mut frame = {
            let body = Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Not(Box::new(Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::Literal(Literal::Bool(false)),
                })),
            };
            let code = Arc::new(comptime_compiler::compile(&body, &mut 100).unwrap());
            vm.bytecode_cache.insert(
                DeclarationId {
                    package: 1,
                    local: 0,
                },
                Some(code.clone()),
            );
            let mut frame = vm.start_frame(&code, &mut environment).unwrap();
            assert!(matches!(
                vm.resume_frame(&mut frame, 1).unwrap(),
                Progress::Suspended
            ));
            frame
        };
        vm.bytecode_cache = CodeCache::default();
        assert!(matches!(
            vm.resume_frame(&mut frame, 100).unwrap(),
            Progress::Complete(Flow::Value(Value::Bool(true)))
        ));
        assert!(frame.restore_environment().is_empty());
    }

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
        let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
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
        let code = Arc::new(comptime_compiler::compile(&body, &mut 100).unwrap());
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
            let code = Arc::new(comptime_compiler::compile(&body, &mut 1000).unwrap());
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
        let code = Arc::new(comptime_compiler::compile(&body, &mut 100).unwrap());
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
                Ok(Progress::Complete(_) | Progress::Call { .. } | Progress::Default { .. }) => {
                    panic!("malformed operation completed")
                }
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
        let code = Arc::new(comptime_compiler::compile(&recovered, &mut 100).unwrap());
        assert!(matches!(
            vm.execute_code(&code, &mut environment).unwrap(),
            Flow::Value(Value::Bool(false))
        ));
    }
}

#[cfg(test)]
#[path = "bytecode_type_equality.rs"]
mod type_equality_tests;

#[cfg(test)]
#[path = "bytecode_type_constructors.rs"]
mod type_constructor_tests;

#[cfg(test)]
#[path = "bytecode_object_borrows.rs"]
mod object_borrow_tests;

#[cfg(test)]
#[path = "bytecode_lists.rs"]
mod list_tests;

#[cfg(test)]
#[path = "bytecode_compilation.rs"]
mod compilation_tests;

#[cfg(test)]
#[path = "normal_types/tests.rs"]
mod normal_type_tests;

#[cfg(test)]
#[path = "input_preparation_tests.rs"]
mod input_preparation_tests;
