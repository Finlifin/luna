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
    let report = run_report_filtered(package, identity, fuel, inline, protocols, filter)?;
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

pub fn run_report_filtered(
    package: &Package,
    identity: u64,
    fuel: u64,
    inline: bool,
    protocols: Option<&elaboration::program::Protocols>,
    filter: Option<&str>,
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
    let mut checked = None;
    let mut reference = None;
    for function in functions {
        let label = test_label(function);
        if filter.is_some_and(|text| !label.contains(text)) {
            report.filtered_out += 1;
            continue;
        }
        let result = if let Some((copy, drop)) = selected {
            let session = checked.get_or_insert_with(|| {
                crate::execution::CheckedSession::new(
                    std::borrow::Cow::Borrowed(package),
                    crate::resources::Selection::Identity(copy),
                    crate::resources::Selection::Identity(drop),
                    fuel,
                    inline,
                )
            });
            match session {
                Ok(session) => session.invoke(function.declaration, &function.name),
                Err(error) => Err(error.clone()),
            }
        } else {
            let session = reference.get_or_insert_with(|| {
                crate::execution::ReferenceSession::new(std::borrow::Cow::Borrowed(package), fuel)
            });
            match session {
                Ok(session) => session.invoke(function.declaration, &function.name),
                Err(error) => Err(error.clone()),
            }
        };
        let failure = match result {
            Ok(comptime_interpretor::Value::Unit) => None,
            Ok(_) => Some("test must return void".into()),
            Err(error) => Some(error),
        };
        report.results.push(TestResult {
            declaration: function.declaration,
            name: label.to_owned(),
            failure,
        });
    }
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
    if fuel == 0 {
        return Err("test fuel must be positive".into());
    }
    if timeout.is_zero() {
        return Err("native test timeout must be positive".into());
    }
    if inline && protocols.is_none() {
        return Err("inline native tests require Copy and Drop bindings".into());
    }
    let mut expanded = None;
    let selected = protocols
        .map(|protocols| {
            Ok::<_, String>((
                protocols.copy.ok_or("missing test Copy binding")?,
                protocols.drop.ok_or("missing test Drop binding")?,
            ))
        })
        .transpose()?;
    let mut report = TestReport::default();
    for function in test_functions(package, identity)? {
        let label = test_label(function);
        if filter.is_some_and(|filter| !label.contains(filter)) {
            report.filtered_out += 1;
            continue;
        }
        let result = if matches!(function.signature.callable_signature(), nir::Term::Function { result, .. }
            if **result == nir::Term::Primitive(nir::Primitive::Void))
        {
            let input = if inline {
                expanded
                    .get_or_insert_with(|| crate::host::inline_package(package, fuel))
                    .as_ref()
                    .map_err(Clone::clone)
            } else {
                Ok(package)
            };
            input
                .and_then(|input| {
                    crate::host::llvm_ir_declaration(input, function.declaration, fuel, selected)
                })
                .and_then(|ir| toolchain.execute_test(&ir, timeout))
        } else {
            Err("test must return void".into())
        };
        report.results.push(TestResult {
            declaration: function.declaration,
            name: label.into(),
            failure: result.err(),
        });
    }
    Ok(report)
}
