use super::*;

fn duplicate(request: &Preparation) -> Preparation {
    let Operation::Call {
        code,
        instruction,
        dynamic_method,
    } = &request.operation
    else {
        panic!("ordinary call fixture required");
    };
    assert!(dynamic_method.is_none());
    let operation = Operation::Call {
        code: code.clone(),
        instruction: *instruction,
        dynamic_method: None,
    };
    Preparation {
        ticket: request.ticket.clone(),
        operation,
    }
}

fn fixture(repeat: bool) -> (Arc<Program>, Environment) {
    let source = SourceRange { start: 1, end: 2 };
    let function = LocalId { owner: 1, index: 0 };
    let boolean = Term::Primitive(Primitive::Bool);
    let signature = Term::ComptimeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(boolean.clone()),
    }));
    let closure = Value::Closure(Arc::new(Closure {
        definition_source: None,
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
    let call = Expr {
        source,
        ty: boolean,
        kind: K::Call {
            callee: Box::new(Expr {
                source,
                ty: signature,
                kind: K::Local(function),
            }),
            arguments: vec![],
            env_arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    let body = if repeat {
        Expr {
            source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Loop {
                identity: LocalId { owner: 1, index: 1 },
                body: Box::new(call),
            },
        }
    } else {
        call
    };
    (
        Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap()),
        Environment::from([(function, closure)]),
    )
}

fn request(vm: &mut Interpreter<'_>, frame: &mut Frame) -> Preparation {
    let Progress::Prepare(request) = vm.resume_frame(frame, 1000).unwrap() else {
        panic!("expected preparation");
    };
    request
}

fn reject(vm: &mut Interpreter<'_>, frame: &mut Frame, request: Preparation) {
    let state = frame.to_string();
    let fuel = vm.remaining_steps();
    assert_eq!(
        vm.prepare_bytecode_request(frame, request)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidControlFlow
    );
    assert_eq!(frame.to_string(), state);
    assert_eq!(vm.remaining_steps(), fuel);
}

#[test]
fn shared_code_activations_reject_cross_delivery_before_consuming_callees() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let (code, environment) = fixture(false);
    let mut first = vm.start_frame(&code, &mut environment.clone()).unwrap();
    let mut second = vm.start_frame(&code, &mut environment.clone()).unwrap();
    let first_request = request(&mut vm, &mut first);
    let second_request = request(&mut vm, &mut second);
    assert_eq!(first.pc, second.pc);
    reject(&mut vm, &mut first, duplicate(&second_request));
    reject(&mut vm, &mut second, duplicate(&first_request));
    for (mut frame, request) in [(first, first_request), (second, second_request)] {
        vm.prepare_bytecode_request(&mut frame, request).unwrap();
        assert_eq!(frame.calls.len(), 1);
        let Progress::Call { callee, arguments } = vm.resume_frame(&mut frame, 1000).unwrap()
        else {
            panic!("expected call");
        };
        let value = vm.invoke_value_scoped(callee, arguments, None).unwrap();
        frame.deliver(value).unwrap();
        assert!(matches!(
            vm.resume_frame(&mut frame, 1000).unwrap(),
            Progress::Complete(Flow::Value(Value::Bool(true)))
        ));
    }
}

#[test]
fn repeated_instruction_requires_a_new_ticket_and_cancel_retires_it() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let (code, mut environment) = fixture(true);
    let mut frame = vm.start_frame(&code, &mut environment).unwrap();
    let first = request(&mut vm, &mut frame);
    let stale = duplicate(&first);
    let pc = frame.pc;
    vm.prepare_bytecode_request(&mut frame, first).unwrap();
    reject(&mut vm, &mut frame, duplicate(&stale));
    let Progress::Call { callee, arguments } = vm.resume_frame(&mut frame, 1000).unwrap() else {
        panic!("expected call");
    };
    frame
        .deliver(vm.invoke_value_scoped(callee, arguments, None).unwrap())
        .unwrap();
    let second = request(&mut vm, &mut frame);
    assert_eq!(frame.pc, pc, "revisit the same instruction in a real loop");
    assert!(!second.ticket.matches(&stale.ticket));
    reject(&mut vm, &mut frame, stale);
    // Moving a frame changes neither its authority nor its current request.
    let mut frame = Box::new(frame);
    vm.prepare_bytecode_request(&mut frame, duplicate(&second))
        .unwrap();
    reject(&mut vm, &mut frame, duplicate(&second));
    let Progress::Call { callee, arguments } = vm.resume_frame(&mut frame, 1000).unwrap() else {
        panic!("expected second call");
    };
    frame
        .deliver(vm.invoke_value_scoped(callee, arguments, None).unwrap())
        .unwrap();
    let third = request(&mut vm, &mut frame);
    frame.cancel();
    reject(&mut vm, &mut frame, third);
    reject(&mut vm, &mut frame, second);
    assert_eq!(
        vm.resume_frame(&mut frame, 1000).unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
}

#[test]
fn malformed_owned_requests_fail_closed_without_replaying_or_consuming_operands() {
    for wrong_code in [false, true] {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let (code, mut environment) = fixture(false);
        let mut frame = vm.start_frame(&code, &mut environment).unwrap();
        let mut invalid = request(&mut vm, &mut frame);
        let original = duplicate(&invalid);
        let slots = frame.slots.clone();
        let Operation::Call {
            code, instruction, ..
        } = &mut invalid.operation
        else {
            unreachable!()
        };
        if wrong_code {
            *code = fixture(false).0;
        } else {
            *instruction += 1;
        }
        assert_eq!(
            vm.prepare_bytecode_request(&mut frame, invalid)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(frame.slots, slots);
        assert!(frame.terminal);
        reject(&mut vm, &mut frame, original);
    }
}

#[test]
fn failed_preparation_cannot_resume_or_retry_after_consuming_its_operand() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let (code, mut environment) = fixture(false);
    let mut frame = vm.start_frame(&code, &mut environment).unwrap();
    let pending = request(&mut vm, &mut frame);
    let replay = duplicate(&pending);
    let Operation::Call { instruction, .. } = &pending.operation else {
        unreachable!()
    };
    let Op::BeginCall { callee, .. } = &code.instructions()[*instruction].op else {
        unreachable!()
    };
    frame.slots[*callee] = Some(Value::Bool(false));
    assert!(vm.prepare_bytecode_request(&mut frame, pending).is_err());
    assert!(frame.slots[*callee].is_none());
    assert!(frame.terminal);
    reject(&mut vm, &mut frame, replay);
    assert_eq!(
        vm.resume_frame(&mut frame, 1000).unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
}

#[test]
fn generation_exhaustion_never_wraps_and_dumps_do_not_expose_owner_addresses() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let (code, mut environment) = fixture(false);
    let mut frame = vm.start_frame(&code, &mut environment).unwrap();
    frame.preparations.set_generation(u64::MAX - 1);
    let pending = request(&mut vm, &mut frame);
    let Operation::Call { instruction, .. } = pending.operation else {
        unreachable!()
    };
    assert_eq!(
        pending.to_string(),
        format!(
            "(preparation-request (generation {}) (prepare-call (instruction {instruction}) (dynamic-method)))",
            u64::MAX
        )
    );
    assert_eq!(
        frame.preparations.issue().unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
    assert_eq!(frame.preparations.generation(), u64::MAX);
    vm.prepare_bytecode_request(&mut frame, pending).unwrap();
}
