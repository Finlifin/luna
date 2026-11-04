//! Compile public source against the real built-in packages and execute concrete IIR.
use comptime_interpretor::Value;
use luna::{
    cli::{self, Command},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn compile(source: &str) -> Result<nir::program::Package, String> {
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
    let result = frontend::compile(&options);
    std::fs::remove_file(path).unwrap();
    match result {
        Ok(Artifact::Nir(package)) => Ok(package),
        Ok(_) => panic!("NIR artifact"),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn run(source: &str) -> Value {
    let package = compile(source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    vm.invoke_instance(&graph.roots[0], vec![]).unwrap()
}
