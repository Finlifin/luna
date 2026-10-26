//! Shared source compilation and interpreter assertions for execution suites.
use comptime_interpretor::{Event, Interpreter};
use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use nir::program::Package;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn compile(source: &str) -> Package {
    compile_with_protocols(source, &Default::default()).unwrap()
}
/// Instantiate checked source and execute its concrete IIR entry.
pub(crate) fn run_iir(
    source: &str,
    entry: &str,
    inputs: Vec<comptime_interpretor::Value>,
) -> comptime_interpretor::Value {
    let package = compile(source);
    let graph = luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], inputs).unwrap()
}
pub(crate) fn compile_with_protocols(
    source: &str,
    protocols: &elaboration::program::Protocols,
) -> Result<Package, String> {
    compile_with_bindings(source, protocols, None)
}
pub(crate) fn compile_with_bindings(
    source: &str,
    protocols: &elaboration::program::Protocols,
    selector: Option<&str>,
) -> Result<Package, String> {
    compile_with_source_bindings(
        source,
        protocols,
        luna::protocols::type_constructors::Selectors {
            splice: selector,
            ..Default::default()
        },
    )
}
pub(crate) fn compile_with_source_bindings(
    source: &str,
    protocols: &elaboration::program::Protocols,
    selectors: luna::protocols::type_constructors::Selectors<'_>,
) -> Result<Package, String> {
    compile_with_source_bindings_for_target(source, protocols, selectors, None)
}

pub(crate) fn compile_with_source_bindings_for_target(
    source: &str,
    protocols: &elaboration::program::Protocols,
    selectors: luna::protocols::type_constructors::Selectors<'_>,
    target: Option<nir::target::TargetData>,
) -> Result<Package, String> {
    let path = std::env::temp_dir().join(format!(
        "luna-execution-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile_bound_for_target(
        &Options {
            input: path.clone(),
            emit: Emit::Nir,
            output: None,
            sysroot: None,
            splice_protocol: selectors.splice.map(str::to_owned),
            meta_list_type: selectors.meta_list.map(str::to_owned),
            tuple_type: selectors.tuple.map(str::to_owned),
            index_protocol: selectors.index.map(str::to_owned),
            ord_protocol: selectors.ordering.map(str::to_owned),
            eq_protocol: selectors.equality.map(str::to_owned),
            list_pattern_protocol: selectors.list_pattern.map(str::to_owned),
            literal_into_protocol: selectors.literal_into.map(str::to_owned),
            arithmetic_protocols: selectors.arithmetic.map(|value| value.map(str::to_owned)),
        },
        0,
        protocols,
        selectors,
        target,
    )
    .map(|(artifact, _)| artifact);
    std::fs::remove_file(path).unwrap();
    let Artifact::Nir(package) = result? else {
        panic!("wrong artifact")
    };
    Ok(package)
}
pub(crate) fn id(package: &Package, name: &str) -> nir::DeclarationId {
    package
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .declaration
}
pub(crate) fn calls(package: &Package, interpreter: &Interpreter<'_>) -> Vec<String> {
    interpreter
        .events()
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(id) => Some(
                package
                    .functions
                    .iter()
                    .find(|f| f.declaration == *id)
                    .unwrap()
                    .name
                    .clone(),
            ),
            _ => None,
        })
        .collect()
}

/// Compile the shared splice fixture with its explicit Tuple declarations.
pub(crate) fn compile_splice_fixture(
    source: &str,
    protocols: &elaboration::program::Protocols,
    splice: Option<&str>,
) -> Result<nir::program::Package, String> {
    compile_with_source_bindings(
        source,
        protocols,
        luna::protocols::type_constructors::Selectors {
            splice,
            meta_list: Some("input.TupleElements"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
}

// Explicit source declarations and protocol identities for this suite. The
// fixture text is compiled as written; no legacy tuple syntax is rewritten.
pub(crate) fn compile_with_tuple_protocols(
    source: &str,
    protocols: &elaboration::program::Protocols,
) -> Result<nir::program::Package, String> {
    compile_with_tuple_selectors(source, protocols, Default::default())
}

pub(crate) fn compile_with_tuple_selectors(
    source: &str,
    protocols: &elaboration::program::Protocols,
    mut selectors: luna::protocols::type_constructors::Selectors<'_>,
) -> Result<nir::program::Package, String> {
    assert!(selectors.meta_list.is_none() && selectors.tuple.is_none());
    selectors.meta_list = Some("input.TupleElements");
    selectors.tuple = Some("input.Tuple");
    compile_with_source_bindings(
        &format!(
            "pure comptime fn TupleElements<T>->.Type'builtin {{T}}
            pure comptime fn Tuple<...Es:TupleElements<.Type'builtin>>->.Type'builtin {{bool}}
            {source}"
        ),
        protocols,
        selectors,
    )
}

pub(crate) fn compile_with_tuple(source: &str) -> nir::program::Package {
    compile_with_tuple_protocols(source, &Default::default()).unwrap()
}

/// Execute a fixture using explicit Tuple and meta.List type constructors.
pub(crate) fn run_iir_with_tuple(
    source: &str,
    entry: &str,
    inputs: Vec<comptime_interpretor::Value>,
) -> comptime_interpretor::Value {
    let package = compile_with_tuple(source);
    let graph = luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], inputs).unwrap()
}
