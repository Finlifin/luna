use super::*;
use comptime_compiler::definitions::Kind as BuildKind;
use std::cell::Cell;

fn list(element: Term) -> Term {
    Term::NormalForm {
        head: id(80),
        sort: nir::Sort::Type,
        arguments: vec![element],
    }
}

fn world() -> Package {
    Package {
        normal_forms: vec![Head {
            declaration: id(80),
            collector: None,
            role: Role::TypeList,
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: id(81),
                binding: local(80),
                kind: Kind::Type,
                ty: ty(Primitive::Type),
            }],
        }],
        ..Default::default()
    }
}

fn symbols(names: &[&str]) -> Value {
    Value::List {
        ty: list(ty(Primitive::Symbol)),
        values: names
            .iter()
            .map(|name| Value::Symbol((*name).into()))
            .collect(),
    }
}

fn packet(kind: BuildKind) -> (Term, Value) {
    let symbol = ty(Primitive::Symbol);
    let types = list(ty(Primitive::Type));
    let symbols_ty = list(symbol.clone());
    let empty_types = || Value::List {
        ty: types.clone(),
        values: vec![].into(),
    };
    let (domains, values) = match kind {
        BuildKind::Newtype => (
            vec![symbol, ty(Primitive::Type)],
            vec![
                Value::Symbol("Generated".into()),
                Value::Type(ty(Primitive::Bool)),
            ],
        ),
        BuildKind::Struct => (
            vec![symbol, symbols_ty, types.clone()],
            vec![
                Value::Symbol("Generated".into()),
                symbols(&[]),
                empty_types(),
            ],
        ),
        BuildKind::Enum => (
            vec![
                symbol,
                symbols_ty.clone(),
                symbols_ty.clone(),
                symbols_ty.clone(),
                list(symbols_ty.clone()),
                list(types.clone()),
            ],
            vec![
                Value::Symbol("Generated".into()),
                symbols(&[]),
                symbols(&["ready"]),
                symbols(&["unit"]),
                Value::List {
                    ty: list(symbols_ty),
                    values: vec![symbols(&[])].into(),
                },
                Value::List {
                    ty: list(types.clone()),
                    values: vec![empty_types()].into(),
                },
            ],
        ),
    };
    (Term::Tuple(domains), Value::Tuple(values))
}

fn assign(index: u32, value: Expr) -> Expr {
    expr(
        ty(Primitive::Void),
        K::Assign {
            place: Place {
                root: local(index),
                fields: vec![],
            },
            value: Box::new(value),
        },
    )
}

fn fixture(kind: BuildKind) -> (Arc<comptime_compiler::Program>, Environment) {
    let (domain, value) = packet(kind);
    let operand = Box::new(expr(
        domain.clone(),
        K::Block {
            statements: vec![assign(
                1,
                expr(
                    ty(Primitive::Bool),
                    K::Not(Box::new(expr(ty(Primitive::Bool), K::Local(local(1))))),
                ),
            )],
            result: Some(Box::new(expr(domain, K::Local(local(0))))),
        },
    ));
    let build = expr(
        ty(Primitive::Type),
        match kind {
            BuildKind::Struct => K::BuildStruct(operand),
            BuildKind::Enum => K::BuildEnum(operand),
            BuildKind::Newtype => K::BuildNewtype(operand),
        },
    );
    let body = expr(
        ty(Primitive::Type),
        K::Block {
            statements: vec![
                expr(
                    ty(Primitive::Void),
                    K::Let {
                        binding: local(3),
                        value: Box::new(build),
                    },
                ),
                assign(
                    2,
                    expr(ty(Primitive::Bool), K::Literal(Literal::Bool(true))),
                ),
            ],
            result: Some(Box::new(expr(ty(Primitive::Type), K::Local(local(3))))),
        },
    );
    (
        Arc::new(comptime_compiler::compile(&body, &mut 100_000).unwrap()),
        Environment::from([
            (local(0), value),
            (local(1), Value::Bool(false)),
            (local(2), Value::Bool(false)),
        ]),
    )
}

#[derive(Clone, Copy)]
enum Failure {
    None,
    Prepare,
    PreparePanic,
    Commit,
    CommitPanic,
}

struct Host {
    preparations: Cell<usize>,
    commits: Cell<usize>,
    failure: Cell<Failure>,
}

impl Host {
    fn new(failure: Failure) -> Self {
        Self {
            preparations: Cell::new(0),
            commits: Cell::new(0),
            failure: Cell::new(failure),
        }
    }

    fn prepare(&self) -> Result<(), Error> {
        self.preparations.set(self.preparations.get() + 1);
        match self.failure.get() {
            Failure::Prepare => Err(Error {
                kind: ErrorKind::Type,
                source: Some(SOURCE),
            }),
            Failure::PreparePanic => panic!("host preparation interrupted"),
            _ => Ok(()),
        }
    }

    fn commit(&self) -> Result<(), Error> {
        self.commits.set(self.commits.get() + 1);
        match self.failure.get() {
            Failure::Commit => Err(Error {
                kind: ErrorKind::Type,
                source: Some(SOURCE),
            }),
            Failure::CommitPanic => panic!("host commit interrupted"),
            _ => Ok(()),
        }
    }

    fn structure(&self, request: &StructBuildRequest) -> Result<PreparedStructBuild<'_>, Error> {
        self.prepare()?;
        Ok(PreparedStructBuild {
            result: id(90),
            definitions: vec![nir::program::Struct {
                declaration: id(90),
                name: request.name.clone(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: request
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect(),
                packed: nir::program::PackedMode::None,
            }]
            .into(),
            commit: Box::new(|| self.commit()),
        })
    }

    fn enumeration(&self, request: &EnumBuildRequest) -> Result<PreparedEnumBuild<'_>, Error> {
        self.prepare()?;
        Ok(PreparedEnumBuild {
            result: id(90),
            definitions: vec![nir::program::Enum {
                declaration: id(90),
                name: request.name.clone(),
                is_intrinsic: false,
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                groups: request.groups.clone(),
                variants: request.variants.clone(),
                encoding: request.encoding.clone(),
                symbol_encoding: request.symbol_encoding.clone(),
            }]
            .into(),
            commit: Box::new(|| self.commit()),
        })
    }

    fn newtype(&self, request: &NewtypeBuildRequest) -> Result<PreparedNewtypeBuild<'_>, Error> {
        self.prepare()?;
        Ok(PreparedNewtypeBuild {
            result: id(90),
            definitions: vec![nir::newtypes::Newtype {
                declaration: id(90),
                name: request.name.clone(),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: request.representation.clone(),
            }]
            .into(),
            commit: Box::new(|| self.commit()),
        })
    }
}

fn await_definition(session: &mut Session<'_, '_, '_>) {
    for _ in 0..32 {
        assert!(matches!(session.poll().unwrap(), Poll::Pending));
        if matches!(&session.pending, Some(Pending::Bytecode(request))
            if request.to_string().contains("(prepare-definition-build "))
        {
            assert!(
                session
                    .to_string()
                    .contains("(prepare-definition-build (instruction ")
            );
            return;
        }
    }
    panic!("definition request was not yielded");
}

#[test]
fn all_definition_builders_suspend_after_operands_and_commit_once() {
    for kind in [BuildKind::Struct, BuildKind::Enum, BuildKind::Newtype] {
        for cancel in [true, false] {
            let package = world();
            let host = Host::new(Failure::None);
            let structure = |request: &StructBuildRequest, _: &mut u64| host.structure(request);
            let enumeration = |request: &EnumBuildRequest, _: &mut u64| host.enumeration(request);
            let newtype = |request: &NewtypeBuildRequest, _: &mut u64| host.newtype(request);
            let mut vm = Interpreter::for_comptime(&package, Budget::default())
                .unwrap()
                .with_struct_builder(&structure)
                .with_enum_builder(&enumeration)
                .with_newtype_builder(&newtype);
            let (code, mut environment) = fixture(kind);
            let mut frame = vm.start_frame(&code, &mut environment).unwrap();
            {
                let mut session = Box::new(Session::fragment(&mut vm, &mut frame));
                await_definition(&mut session);
                assert_eq!(host.preparations.get(), 0);
                assert_eq!(host.commits.get(), 0);
                session = Box::new(*session);
                if !cancel {
                    assert!(
                        matches!((*session).run().unwrap(), Some(Flow::Value(Value::Type(Term::Nominal { declaration, .. }))) if declaration == id(90))
                    );
                    assert_eq!(host.preparations.get(), 1);
                    assert_eq!(host.commits.get(), 1);
                } else {
                    drop(session);
                }
            }
            assert_eq!(
                vm.resume_frame(&mut frame, 1).err().unwrap().kind,
                ErrorKind::InvalidControlFlow
            );
            environment = frame.restore_environment();
            assert_eq!(environment.get(&local(1)), Some(&Value::Bool(true)));
            assert_eq!(environment.get(&local(2)), Some(&Value::Bool(!cancel)));
            assert_eq!(vm.depth, 0);
            assert_eq!(vm.invocation_depth, 0);
        }
    }
}

#[test]
fn definition_host_failure_and_unwind_restore_the_session_before_retry() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for failure in [
        Failure::Prepare,
        Failure::PreparePanic,
        Failure::Commit,
        Failure::CommitPanic,
    ] {
        let package = world();
        let host = Host::new(failure);
        let newtype = |request: &NewtypeBuildRequest, _: &mut u64| host.newtype(request);
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_newtype_builder(&newtype);
        vm.depth = 3;
        vm.effect_state.invocation = 71;
        vm.effect_state.resume = Some(19);
        let (code, mut environment) = fixture(BuildKind::Newtype);
        let mut frame = vm.start_frame(&code, &mut environment).unwrap();
        let result = catch_unwind(AssertUnwindSafe(|| {
            Session::fragment(&mut vm, &mut frame).run()
        }));
        match failure {
            Failure::PreparePanic | Failure::CommitPanic => assert!(result.is_err()),
            _ => assert_eq!(result.unwrap().err().unwrap().kind, ErrorKind::Type),
        }
        assert_eq!(host.preparations.get(), 1);
        assert!(!vm.newtypes.contains_key(&id(90)));
        environment = frame.restore_environment();
        assert_eq!(environment.get(&local(1)), Some(&Value::Bool(true)));
        assert_eq!(environment.get(&local(2)), Some(&Value::Bool(false)));
        assert_eq!(vm.depth, 3);
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.invocation, 71);
        assert_eq!(vm.effect_state.resume, Some(19));
        host.failure.set(Failure::None);
        vm.budget = Budget::default();
        let (code, mut environment) = fixture(BuildKind::Newtype);
        let mut frame = vm.start_frame(&code, &mut environment).unwrap();
        assert!(matches!(
            Session::fragment(&mut vm, &mut frame).run().unwrap(),
            Some(Flow::Value(Value::Type(_)))
        ));
        assert_eq!(host.preparations.get(), 2);
        assert!(vm.newtypes.contains_key(&id(90)));
    }
}

#[test]
fn definition_preparation_rejects_other_activations_of_the_same_code() {
    let package = world();
    let host = Host::new(Failure::None);
    let newtype = |request: &NewtypeBuildRequest, _: &mut u64| host.newtype(request);
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_newtype_builder(&newtype);
    let (code, environment) = fixture(BuildKind::Newtype);
    let mut frame = vm.start_frame(&code, &mut environment.clone()).unwrap();
    let mut foreign = vm.start_frame(&code, &mut environment.clone()).unwrap();
    let bytecode::Progress::Prepare(request) = vm.resume_frame(&mut frame, 100_000).unwrap() else {
        panic!("definition preparation")
    };
    let bytecode::Progress::Prepare(foreign_request) =
        vm.resume_frame(&mut foreign, 100_000).unwrap()
    else {
        panic!("foreign definition preparation")
    };
    let before = frame.to_string();
    assert_eq!(
        vm.prepare_bytecode_request(&mut frame, foreign_request)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidControlFlow
    );
    assert_eq!(
        frame.to_string(),
        before,
        "reject before consuming operands or wait state"
    );
    assert_eq!(host.preparations.get(), 0);
    vm.prepare_bytecode_request(&mut frame, request).unwrap();
    assert_eq!(host.preparations.get(), 1);
    assert_eq!(host.commits.get(), 1);
    assert!(matches!(
        Session::fragment(&mut vm, &mut frame).run().unwrap(),
        Some(Flow::Value(Value::Type(_)))
    ));
    assert_eq!(host.preparations.get(), 1);
    assert_eq!(host.commits.get(), 1);
    foreign.cancel();
}

#[test]
fn exhausted_definition_request_keeps_operand_effects_and_never_enters_the_host() {
    let package = world();
    let host = Host::new(Failure::None);
    let newtype = |request: &NewtypeBuildRequest, _: &mut u64| host.newtype(request);
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_newtype_builder(&newtype);
    let (code, mut environment) = fixture(BuildKind::Newtype);
    let mut frame = vm.start_frame(&code, &mut environment).unwrap();
    let mut session = Session::fragment(&mut vm, &mut frame);
    await_definition(&mut session);
    session.vm.budget.steps = 0;
    assert_eq!(session.run().err().unwrap().kind, ErrorKind::StepsExhausted);
    assert_eq!(host.preparations.get(), 0);
    assert_eq!(host.commits.get(), 0);
    environment = frame.restore_environment();
    assert_eq!(environment.get(&local(1)), Some(&Value::Bool(true)));
    assert_eq!(environment.get(&local(2)), Some(&Value::Bool(false)));
    assert!(!vm.newtypes.contains_key(&id(90)));
    vm.budget = Budget::default();
    let (code, mut environment) = fixture(BuildKind::Newtype);
    assert!(matches!(
        vm.execute_code(&code, &mut environment).unwrap(),
        Flow::Value(Value::Type(_))
    ));
    assert_eq!(host.preparations.get(), 1);
    assert_eq!(host.commits.get(), 1);
}
