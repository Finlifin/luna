//! Compile public source against the real built-in packages and execute concrete IIR.
use comptime_interpretor::{Budget, Interpreter, SourceResolvers, Value};
use elaboration::materialization::{DeclarationSource, SourceInvocationQueries};
use luna::{
    cli::{self, Command},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Covers the sysroot declaration view, phase validation, queries and execution.
/// Callers testing exhaustion supply their own allowance instead.
#[allow(dead_code)]
pub(crate) const COMPTIME_BUDGET: Budget = Budget {
    steps: 1_000_000,
    depth: 128,
};

/// The single-source sysroot fixture assigns package 0 to the user's module.
/// Imported libraries can define functions with the same unqualified names.
#[allow(dead_code)]
pub(crate) fn source_function(package: &nir::program::Package, name: &str) -> nir::DeclarationId {
    let mut matches = package
        .functions
        .iter()
        .filter(|function| function.declaration.package == 0 && function.name == name);
    let selected = matches.next().expect("source function").declaration;
    assert!(matches.next().is_none(), "ambiguous source function {name}");
    selected
}

pub(crate) fn compile(source: &str) -> Result<nir::program::Package, String> {
    let result = with_options(source, frontend::compile);
    match result {
        Ok(Artifact::Nir(package)) => Ok(package),
        Ok(_) => panic!("NIR artifact"),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn invoke(source: &str, entry: &str) -> Result<Value, String> {
    with_options(source, |options| {
        frontend::invoke_comptime(options, entry, 10_000_000, None, None)
    })
}

fn with_options<T>(source: &str, action: impl FnOnce(&cli::Options) -> T) -> T {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-template-locals-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("check command")
    };
    let result = action(&options);
    std::fs::remove_file(path).unwrap();
    result
}

pub(crate) fn run(source: &str) -> Value {
    run_with_inputs(source, "run", vec![])
}

pub(crate) fn run_with_inputs(source: &str, entry: &str, inputs: Vec<Value>) -> Value {
    let package = compile(source).unwrap();
    let graph = luna::instantiation::instantiate(&package, entry, 10_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], inputs).unwrap()
}

/// Keep source queries alive for execution in the exact checked package.
/// Reference routing does not substitute a closed dispatch world or runtime VM.
#[allow(dead_code)]
pub(crate) fn with_comptime<T>(
    package: &nir::program::Package,
    mut budget: Budget,
    reference: bool,
    action: impl FnOnce(&mut Interpreter<'_>) -> T,
) -> T {
    let mut remaining = usize::try_from(budget.steps).unwrap();
    // Source execution selects its declaration view inside the VM, just as the
    // CLI does. Borrow the exact package while all three query services live.
    let source = DeclarationSource::Borrowed(package);
    let context = elaboration::ptc::Context::from_package_metered(package, &mut remaining).unwrap();
    budget.steps = remaining as u64;
    let queries = SourceInvocationQueries::in_source(source, &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    let mut vm = Interpreter::for_comptime_resolving_in_budget(
        package,
        &[],
        &[],
        &[],
        SourceResolvers {
            types: &solver,
            methods: &methods,
            associated: &associated,
        },
        &mut budget,
    )
    .unwrap();
    if reference {
        vm = vm.with_reference_execution();
    }
    action(&mut vm)
}
