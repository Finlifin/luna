//! Bounded, process-local reuse of completed frontend artifacts. Invocation and
//! execution are deliberately outside this cache; their effects must run again.
use crate::{frontend::Artifact, package_cli::Request, packages::CompilationInputs};
use std::{collections::VecDeque, path::PathBuf, sync::Arc};

pub struct CachedCompilation {
    pub artifact: Artifact,
    pub bindings: elaboration::program::Protocols,
    pub inputs: CompilationInputs,
}
pub struct CompilationOutcome {
    pub compilation: Arc<CachedCompilation>,
    pub reused: bool,
}
/// Full requests and the current directory form conservative process-local
/// keys. The running compiler binary fixes its implementation and target model.
/// This is not a serialized cache or a cross-process compiler identity scheme.
pub struct CompilationCache {
    capacity: usize,
    entries: VecDeque<CacheEntry>,
}
struct CacheEntry {
    root: PathBuf,
    request: Request,
    canonical_entries: Vec<PathBuf>,
    compilation: Arc<CachedCompilation>,
}

impl CompilationCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn compile(&mut self, request: &Request) -> Result<CompilationOutcome, String> {
        let target = request.semantic_target()?;
        let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
        // Evict before any filesystem revalidation. A failed entry lookup
        // cannot leave a previously valid artifact available as fallback.
        let previous = self
            .entries
            .iter()
            .position(|entry| entry.root == cwd && entry.request == *request)
            .map(|index| self.entries.remove(index).unwrap());
        let canonical_entries = entry_identities(request)?;
        if let Some(entry) = previous {
            if entry.canonical_entries == canonical_entries
                && current(&entry.compilation.inputs)?
                && match &entry.compilation.artifact {
                    crate::frontend::Artifact::Nir(package) => package.target == target,
                    _ => target.is_none(),
                }
                && entry_identities(request)? == canonical_entries
            {
                protect_output(request, &entry.compilation.inputs)?;
                let compilation = Arc::clone(&entry.compilation);
                self.entries.push_back(entry);
                return Ok(CompilationOutcome {
                    compilation,
                    reused: true,
                });
            }
        }
        let mut inputs = CompilationInputs::default();
        let (artifact, bindings) = request.compile_observed(Some(&mut inputs))?;
        if entry_identities(request)? != canonical_entries || !current(&inputs)? {
            return Err(
                "source inputs changed during compilation; retry with stable inputs".into(),
            );
        }
        protect_output(request, &inputs)?;
        let compilation = Arc::new(CachedCompilation {
            artifact,
            bindings,
            inputs,
        });
        if self.capacity != 0 {
            if self.entries.len() == self.capacity {
                self.entries.pop_front();
            }
            self.entries.push_back(CacheEntry {
                root: cwd,
                request: request.clone(),
                canonical_entries,
                compilation: Arc::clone(&compilation),
            });
        }
        Ok(CompilationOutcome {
            compilation,
            reused: false,
        })
    }
}
// Canonical entry paths determine package ownership exclusions, even for a
// non-root source file whose bytes and parent directory membership stay equal.
fn entry_identities(request: &Request) -> Result<Vec<PathBuf>, String> {
    let dependencies = request
        .dependencies
        .iter()
        .map(|(requester, alias, provider)| resolve::DependencyEdge {
            requester: *requester,
            alias: alias.clone(),
            provider: *provider,
        })
        .collect::<Vec<_>>();
    resolve::validate_package_dependencies(
        request.packages.iter().map(|p| p.resolution_id),
        &dependencies,
        request.builtin,
    )
    .map_err(|error| format!("invalid package graph: {}", error.message()))?;
    request
        .packages
        .iter()
        .map(|package| {
            package.entry.canonicalize().map_err(|error| {
                format!(
                    "cannot revalidate entry for package {} at {:?}: {error}",
                    package.resolution_id, package.entry,
                )
            })
        })
        .collect()
}

fn current(inputs: &CompilationInputs) -> Result<bool, String> {
    // Scan first: newly added sources must invalidate even when every old
    // source still has the same bytes.
    Ok(inputs.changed_discovery()?.is_empty() && inputs.changed_sources()?.is_empty())
}
fn protect_output(request: &Request, inputs: &CompilationInputs) -> Result<(), String> {
    for source in &inputs.sources {
        crate::packages::protect_source_output(&source.path, request.options.output.as_deref())?;
    }
    Ok(())
}
