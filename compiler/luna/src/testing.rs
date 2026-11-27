//! Ordinary source tests support bounded reference execution and native LLVM
//! execution. Test labels never become callable source names.
use nir::{DeclarationId, program::Package};
use std::fmt;

#[derive(Debug)]
pub struct TestResult {
    pub declaration: DeclarationId,
    pub name: String,
    pub failure: Option<String>,
}

#[derive(Debug, Default)]
pub struct TestReport {
    pub results: Vec<TestResult>,
    pub filtered_out: usize,
}

impl TestReport {
    pub fn failures(&self) -> usize {
        self.results
            .iter()
            .filter(|result| result.failure.is_some())
            .count()
    }

    pub fn is_success(&self) -> bool {
        self.failures() == 0
    }
}

impl fmt::Display for TestReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use nir::program::Literal;
        f.write_str("(test-results")?;
        for result in &self.results {
            write!(
                f,
                " ({} (declaration {}) (name {})",
                if result.failure.is_some() {
                    "failed"
                } else {
                    "passed"
                },
                result.declaration,
                Literal::String(result.name.clone())
            )?;
            if let Some(error) = &result.failure {
                write!(f, " (error {})", Literal::String(error.clone()))?;
            }
            f.write_str(")")?;
        }
        write!(
            f,
            " (count {}) (failures {}) (filtered-out {}))",
            self.results.len(),
            self.failures(),
            self.filtered_out
        )
    }
}

/// Compatibility adapter for CLI actions: failure reports go to the error
/// channel and cannot replace an existing successful output artifact.
pub fn run(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
) -> Result<String, String> {
    run_filtered(package, identity, fuel, inline, protocols, None)
}

pub fn run_filtered(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
) -> Result<String, String> {
    run_filtered_with_jobs(package, identity, fuel, inline, protocols, filter, 1)
}

pub fn run_filtered_with_jobs(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
    jobs: usize,
) -> Result<String, String> {
    let report =
        run_report_filtered_with_jobs(package, identity, fuel, inline, protocols, filter, jobs)?;
    if report.is_success() {
        Ok(report.to_string())
    } else {
        Err(format!("source tests failed: {report}"))
    }
}

/// Each test receives a fresh interpreter and per-stage budget. A failed test
/// does not prevent independent tests from producing their own evidence.
pub fn run_report(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
) -> Result<TestReport, String> {
    run_report_filtered(package, identity, fuel, inline, protocols, None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestReturnKind {
    Void,
    ErrorResultVoid,
}

fn classify_test_return(function: &nir::program::Function) -> Option<TestReturnKind> {
    let nir::Term::Function { parameters, result } = function.signature.callable_signature() else {
        return None;
    };
    if !parameters.is_empty() {
        return None;
    }
    match result.as_ref() {
        nir::Term::Primitive(nir::Primitive::Void) => Some(TestReturnKind::Void),
        nir::Term::ErrorResult { result: ok, .. }
            if **ok == nir::Term::Primitive(nir::Primitive::Void) =>
        {
            Some(TestReturnKind::ErrorResultVoid)
        }
        _ => None,
    }
}

pub fn run_report_filtered(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
) -> Result<TestReport, String> {
    run_report_filtered_with_jobs(package, identity, fuel, inline, protocols, filter, 1)
}

pub fn run_report_filtered_with_jobs(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
    jobs: usize,
) -> Result<TestReport, String> {
    if fuel == 0 {
        return Err("test fuel must be positive".into());
    }
    if inline && protocols.is_none() {
        return Err("inline tests require Copy and Drop bindings".into());
    }
    let selected = protocols
        .map(|protocols| {
            Ok::<_, String>((
                protocols.copy.ok_or("missing test Copy binding")?,
                protocols.drop.ok_or("missing test Drop binding")?,
            ))
        })
        .transpose()?;
    let functions = test_functions(package, identity)?;
    let mut report = TestReport::default();
    let mut selected_functions = Vec::new();
    for function in functions {
        let label = test_label(function);
        if filter.is_some_and(|text| !label.contains(text)) {
            report.filtered_out += 1;
            continue;
        }
        selected_functions.push((function, label));
    }
    let (adapted_package, error_adapters) = if selected.is_some()
        && selected_functions.iter().any(|(function, _)| {
            classify_test_return(function) == Some(TestReturnKind::ErrorResultVoid)
        }) {
        let mut adapted = package.clone();
        let mut adapters = std::collections::BTreeMap::new();
        for (function, _) in &selected_functions {
            if classify_test_return(function) == Some(TestReturnKind::ErrorResultVoid) {
                let entry =
                    crate::host::synthesize_error_test_adapter(&mut adapted, function.declaration)?;
                adapters.insert(function.declaration, entry);
            }
        }
        (std::borrow::Cow::Owned(adapted), adapters)
    } else {
        (
            std::borrow::Cow::Borrowed(package),
            std::collections::BTreeMap::new(),
        )
    };
    let run_one_checked = |session: &Result<crate::execution::CheckedSession, String>,
                           function: &nir::program::Function,
                           kind: TestReturnKind|
     -> Option<String> {
        match (session, kind) {
            (Ok(session), TestReturnKind::Void) => {
                match session.invoke(function.declaration, &function.name) {
                    Ok(comptime_interpretor::Value::Unit) => None,
                    Ok(_) => Some("test must return void or !E void".into()),
                    Err(error) => Some(error),
                }
            }
            (Ok(session), TestReturnKind::ErrorResultVoid) => {
                let (adapter, error_descriptions) = &error_adapters[&function.declaration];
                match session.invoke(*adapter, &function.name) {
                    Ok(comptime_interpretor::Value::UnsignedInteger(0)) => None,
                    Ok(comptime_interpretor::Value::UnsignedInteger(tag)) => {
                        let branch = usize::try_from(tag)
                            .ok()
                            .and_then(|value| value.checked_sub(1))
                            .and_then(|index| {
                                error_descriptions
                                    .get(index)
                                    .map(|desc| (index, desc.as_str()))
                            });
                        match branch {
                            Some((index, desc)) => Some(format!(
                                "test returned error (branch (error {index})) {desc}"
                            )),
                            None => Some(format!("unexpected error test tag: {tag}")),
                        }
                    }
                    Ok(other) => Some(format!("unexpected error test adapter result: {other}")),
                    Err(error) => Some(error),
                }
            }
            (Err(error), _) => Some(error.clone()),
        }
    };
    let run_one_reference = |session: &Result<crate::execution::ReferenceSession, String>,
                             function: &nir::program::Function,
                             kind: TestReturnKind|
     -> Option<String> {
        let result = match session {
            Ok(session) => session.invoke(function.declaration, &function.name),
            Err(error) => Err(error.clone()),
        };
        match (result, kind) {
            (Ok(comptime_interpretor::Value::Unit), TestReturnKind::Void) => None,
            (
                Ok(comptime_interpretor::Value::ErrorResult {
                    error: None, value, ..
                }),
                TestReturnKind::ErrorResultVoid,
            ) if *value == comptime_interpretor::Value::Unit => None,
            (
                Ok(comptime_interpretor::Value::ErrorResult {
                    ty,
                    error: Some(index),
                    value,
                }),
                TestReturnKind::ErrorResultVoid,
            ) => {
                let desc = crate::host::describe_error_branch(package, &ty, index);
                Some(format!(
                    "test returned error (branch (error {index})) {desc} (value {value})"
                ))
            }
            (Ok(_), _) => Some("test must return void or !E void".into()),
            (Err(error), _) => Some(error),
        }
    };

    let worker_count = jobs.max(1).min(selected_functions.len().max(1));
    if worker_count <= 1 || selected_functions.len() <= 1 {
        let mut pending_adapted = Some(adapted_package);
        let mut checked = None;
        let mut reference = None;
        for (function, label) in selected_functions {
            let Some(kind) = classify_test_return(function) else {
                report.results.push(TestResult {
                    declaration: function.declaration,
                    name: label.to_owned(),
                    failure: Some("test must return void or !E void".into()),
                });
                continue;
            };
            let failure = if let Some((copy, drop)) = selected {
                let session = checked.get_or_insert_with(|| {
                    let input = pending_adapted
                        .take()
                        .expect("adapted test package available for checked session");
                    crate::execution::CheckedSession::new(
                        input,
                        crate::resources::Selection::Identity(copy),
                        crate::resources::Selection::Identity(drop),
                        fuel,
                        inline,
                    )
                });
                run_one_checked(session, function, kind)
            } else {
                let session = reference.get_or_insert_with(|| {
                    crate::execution::ReferenceSession::new(
                        std::borrow::Cow::Borrowed(package),
                        fuel,
                    )
                });
                run_one_reference(session, function, kind)
            };
            report.results.push(TestResult {
                declaration: function.declaration,
                name: label.to_owned(),
                failure,
            });
        }
        return Ok(report);
    }

    let next_index = std::sync::atomic::AtomicUsize::new(0);
    let adapted_ref = adapted_package.as_ref();
    let selected_ref = &selected_functions;
    let run_one_checked_ref = &run_one_checked;
    let run_one_reference_ref = &run_one_reference;
    let mut worker_results: Vec<Vec<(usize, TestResult)>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let next_index = &next_index;
            let builder = std::thread::Builder::new().stack_size(8 * 1024 * 1024);
            let handle = builder
                .spawn_scoped(scope, move || {
                    let mut local_checked = None;
                    let mut local_reference = None;
                    let mut local_out = Vec::new();
                    loop {
                        let idx = next_index.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= selected_ref.len() {
                            break;
                        }
                        let (function, label) = selected_ref[idx];
                        let Some(kind) = classify_test_return(function) else {
                            local_out.push((
                                idx,
                                TestResult {
                                    declaration: function.declaration,
                                    name: label.to_owned(),
                                    failure: Some("test must return void or !E void".into()),
                                },
                            ));
                            continue;
                        };
                        let failure = if let Some((copy, drop)) = selected {
                            let session = local_checked.get_or_insert_with(|| {
                                crate::execution::CheckedSession::new(
                                    std::borrow::Cow::Borrowed(adapted_ref),
                                    crate::resources::Selection::Identity(copy),
                                    crate::resources::Selection::Identity(drop),
                                    fuel,
                                    inline,
                                )
                            });
                            run_one_checked_ref(session, function, kind)
                        } else {
                            let session = local_reference.get_or_insert_with(|| {
                                crate::execution::ReferenceSession::new(
                                    std::borrow::Cow::Borrowed(package),
                                    fuel,
                                )
                            });
                            run_one_reference_ref(session, function, kind)
                        };
                        local_out.push((
                            idx,
                            TestResult {
                                declaration: function.declaration,
                                name: label.to_owned(),
                                failure,
                            },
                        ));
                    }
                    local_out
                })
                .expect("failed to spawn test worker thread");
            handles.push(handle);
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("test worker thread panicked"))
            .collect()
    });

    let mut ordered: Vec<(usize, TestResult)> = worker_results.drain(..).flatten().collect();
    ordered.sort_by_key(|(idx, _)| *idx);
    report.results = ordered.into_iter().map(|(_, r)| r).collect();
    Ok(report)
}

fn test_functions(
    package: &Package,
    identity: u64,
) -> Result<Vec<&nir::program::Function>, String> {
    // Index once, but diagnose only declarations selected by primary identity.
    // Keep the tests vector as the authority for stable execution order.
    let mut definitions = std::collections::BTreeMap::new();
    for function in &package.functions {
        definitions
            .entry(function.declaration)
            .and_modify(|value| *value = None)
            .or_insert(Some(function));
    }
    let mut seen = std::collections::BTreeSet::new();
    package
        .tests
        .iter()
        .filter(|id| id.package == identity)
        .map(|id| {
            if !seen.insert(*id) {
                return Err("duplicate source test identity".to_owned());
            }
            let function = definitions
                .get(id)
                .ok_or("test declaration has no executable body")?
                .ok_or("test declaration has ambiguous executable bodies")?;
            if function.phase != nir::program::FunctionPhase::Runtime
                || !function.type_parameters.is_empty()
                || !function.parameters.is_empty()
                || !function.env_parameters.is_empty()
            {
                return Err("test must be a non-generic zero-argument runtime function".into());
            }
            Ok(function)
        })
        .collect::<Result<Vec<_>, String>>()
}

#[derive(Debug)]
pub struct TestEntry {
    pub declaration: DeclarationId,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct TestInventory {
    pub entries: Vec<TestEntry>,
    pub filtered_out: usize,
}

impl fmt::Display for TestInventory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(test-inventory")?;
        for entry in &self.entries {
            write!(
                f,
                " (test (declaration {}) (name {}))",
                entry.declaration,
                nir::program::Literal::String(entry.name.clone())
            )?;
        }
        write!(
            f,
            " (count {}) (filtered-out {}))",
            self.entries.len(),
            self.filtered_out
        )
    }
}

/// Discover validated primary-package test identities without execution.
pub fn discover(
    package: &Package,
    identity: u64,
    filter: Option<&str>,
) -> Result<TestInventory, String> {
    let mut inventory = TestInventory::default();
    for function in test_functions(package, identity)? {
        let name = test_label(function);
        if filter.is_some_and(|text| !name.contains(text)) {
            inventory.filtered_out += 1;
        } else {
            inventory.entries.push(TestEntry {
                declaration: function.declaration,
                name: name.into(),
            });
        }
    }
    Ok(inventory)
}

fn test_label(function: &nir::program::Function) -> &str {
    function
        .name
        .rsplit('$')
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or("anonymous")
}

/// LLVM execution of ordinary source tests, isolated by declaration identity.
/// Each runtime deadline starts after compilation; a failed test does not stop
/// the remaining selected tests. Native stdout is not the structured report.
pub fn run_native_report_filtered(
    package: &Package,
    identity: u64,
    fuel: u64,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
    toolchain: &crate::host::Toolchain,
    timeout: std::time::Duration,
) -> Result<TestReport, String> {
    run_native_report_filtered_with_inlining(
        package, identity, fuel, protocols, filter, toolchain, timeout, false,
    )
}

/// Native tests share a lazily expanded source copy, but regenerate each test's
/// checked instance and cleanup schedule by declaration identity.
pub fn run_native_report_filtered_with_inlining(
    package: &Package,
    identity: u64,
    fuel: u64,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
    toolchain: &crate::host::Toolchain,
    timeout: std::time::Duration,
    inline: bool,
) -> Result<TestReport, String> {
    run_native_report_filtered_with_inlining_and_jobs(
        package, identity, fuel, protocols, filter, toolchain, timeout, inline, 1,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_native_report_filtered_with_inlining_and_jobs(
    package: &Package,
    identity: u64,
    fuel: u64,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
    toolchain: &crate::host::Toolchain,
    timeout: std::time::Duration,
    inline: bool,
    jobs: usize,
) -> Result<TestReport, String> {
    if fuel == 0 {
        return Err("test fuel must be positive".into());
    }
    if timeout.is_zero() {
        return Err("native test timeout must be positive".into());
    }
    if inline && protocols.is_none() {
        return Err("inline native tests require Copy and Drop bindings".into());
    }
    let selected = protocols
        .map(|protocols| {
            Ok::<_, String>((
                protocols.copy.ok_or("missing test Copy binding")?,
                protocols.drop.ok_or("missing test Drop binding")?,
            ))
        })
        .transpose()?;
    let mut report = TestReport::default();
    let mut selected_functions = Vec::new();
    for function in test_functions(package, identity)? {
        let label = test_label(function);
        if filter.is_some_and(|filter| !label.contains(filter)) {
            report.filtered_out += 1;
            continue;
        }
        selected_functions.push((function, label));
    }

    let expanded = if inline
        && selected_functions
            .iter()
            .any(|(function, _)| classify_test_return(function).is_some())
    {
        Some(crate::host::inline_package(package, fuel))
    } else {
        None
    };

    let run_one = |function: &nir::program::Function, label: &str| -> TestResult {
        let result = if classify_test_return(function).is_some() {
            let input = match &expanded {
                Some(Ok(pkg)) => Ok(pkg),
                Some(Err(err)) => Err(err.clone()),
                None => Ok(package),
            };
            input
                .and_then(|input| {
                    crate::host::llvm_ir_test_declaration(
                        input,
                        function.declaration,
                        fuel,
                        selected,
                    )
                })
                .and_then(|ir| toolchain.execute_test(&ir, timeout))
        } else {
            Err("test must return void or !E void".into())
        };
        TestResult {
            declaration: function.declaration,
            name: label.into(),
            failure: result.err(),
        }
    };

    let worker_count = jobs.max(1).min(selected_functions.len().max(1));
    if worker_count <= 1 || selected_functions.len() <= 1 {
        for (function, label) in selected_functions {
            report.results.push(run_one(function, label));
        }
        return Ok(report);
    }

    let results = crate::package_cache::parallel_map_ordered(
        &selected_functions,
        worker_count,
        |&(function, label)| Ok::<_, String>(run_one(function, label)),
    )?;
    report.results = results;
    Ok(report)
}
