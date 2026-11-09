//! Immutable declaration metadata is owned by the exact source snapshot.
use super::*;
use std::sync::{Arc, OnceLock};

/// A retained source and its lazily completed declaration directory. The
/// directory contains no query answers, premise contexts or VM state; source
/// bodies and executable contracts remain opaque to directory construction.
#[derive(Clone)]
pub struct DeclarationSnapshot(Arc<SnapshotData>);

struct SnapshotData {
    package: Arc<n::Package>,
    metadata: OnceLock<Arc<DeclarationMetadata>>,
}

impl DeclarationSnapshot {
    pub fn new(package: Arc<n::Package>) -> Self {
        Self(Arc::new(SnapshotData {
            package,
            metadata: OnceLock::new(),
        }))
    }

    pub fn package(&self) -> &Arc<n::Package> {
        &self.0.package
    }

    /// Complete the immutable directory before publishing a prepared owner.
    /// Failure leaves the snapshot unprepared and permits a later retry.
    pub fn prepare(&self, budget: &mut Budget) -> Result<(), AdmissionError> {
        self.ensure_metadata(budget).map(|_| ())
    }

    /// Borrow one source-bound implementation view. Premises stay with the
    /// caller and never enter the retained directory.
    pub fn capabilities<'a>(
        &'a self,
        context: &'a Context,
        assumptions: &'a [n::TraitRequirement],
        budget: &mut Budget,
    ) -> Result<crate::capabilities::World<'a>, AdmissionError> {
        self.ensure_metadata(budget)?;
        Ok(crate::capabilities::World {
            index: &self.0.metadata.get().unwrap().index,
            implementations: &self.0.package.implementations,
            traits: &self.0.package.traits,
            assumptions,
            context,
        })
    }

    fn metadata(&self, budget: &mut Budget) -> Result<Arc<DeclarationMetadata>, AdmissionError> {
        if let Some(metadata) = self.0.metadata.get() {
            construction::spend(&mut budget.steps, 1)?;
            return Ok(Arc::clone(metadata));
        }
        Ok(Arc::clone(self.ensure_metadata(budget)?))
    }

    fn ensure_metadata(
        &self,
        budget: &mut Budget,
    ) -> Result<&Arc<DeclarationMetadata>, AdmissionError> {
        if self.0.metadata.get().is_none() {
            let metadata = DeclarationMetadata::build(&self.0.package, budget)?;
            // Only completed metadata is installed. Concurrent constructors, if
            // used, can retain either complete directory of this same source.
            let _ = self.0.metadata.set(metadata);
        }
        Ok(self.0.metadata.get().unwrap())
    }
}

/// The source and its directory cannot be supplied independently. Borrowed
/// scratch sources build privately; retained snapshots share only metadata.
#[derive(Clone, Copy)]
pub enum DeclarationSource<'a> {
    Borrowed(&'a n::Package),
    Snapshot(&'a DeclarationSnapshot),
}
impl<'a> DeclarationSource<'a> {
    pub fn package(self) -> &'a n::Package {
        match self {
            Self::Borrowed(package) => package,
            Self::Snapshot(snapshot) => snapshot.package(),
        }
    }

    pub fn world(
        self,
        budget: &mut Budget,
        context: Option<(&Context, &[n::TraitRequirement])>,
    ) -> Result<DeclarationWorld<'a>, AdmissionError> {
        DeclarationWorld::from_source_metered(self, budget, context)
    }

    pub(super) fn metadata(
        self,
        budget: &mut Budget,
    ) -> Result<Arc<DeclarationMetadata>, AdmissionError> {
        match self {
            Self::Borrowed(package) => DeclarationMetadata::build(package, budget),
            Self::Snapshot(snapshot) => snapshot.metadata(budget),
        }
    }
}

pub(super) struct DeclarationMetadata {
    pub index: Index,
    pub declaration_parameter_sorts: BTreeMap<Id, nir::Sort>,
    pub callable_functions: HashMap<Id, usize>,
    pub runtime_callables: HashMap<Id, usize>,
    pub normal_heads: HashMap<Id, usize>,
}
impl DeclarationMetadata {
    fn build(package: &n::Package, budget: &mut Budget) -> Result<Arc<Self>, AdmissionError> {
        let index = Index::from_package_metered(package, budget)?;
        let input_fuel = &mut budget.steps;
        // Immutable declaration metadata is indexed once per world. No function
        // invocation result, predicate outcome or mutable state is cached here.
        let mut callable_functions = HashMap::new();
        let mut runtime_callables = HashMap::new();
        let mut normal_heads = HashMap::new();
        for (slot, function) in package.functions.iter().enumerate() {
            *input_fuel = input_fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if callable_functions
                .insert(function.declaration, slot)
                .is_some()
            {
                return Err("duplicate callable declaration".into());
            }
        }
        for (slot, identity) in package.runtime_function_identities.iter().enumerate() {
            *input_fuel = input_fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if runtime_callables
                .insert(identity.declaration, slot)
                .is_some()
            {
                return Err("duplicate runtime callable identity".into());
            }
        }
        for (slot, head) in package.normal_forms.iter().enumerate() {
            *input_fuel = input_fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if normal_heads.insert(head.declaration, slot).is_some() {
                return Err("duplicate normal-form callable identity".into());
            }
        }

        construction::spend(input_fuel, index.parameter_domains().len())?;
        let declaration_parameter_sorts = index
            .parameter_domains()
            .iter()
            .map(|(id, domain)| {
                (
                    *id,
                    if *domain == Term::Primitive(nir::Primitive::Type) {
                        nir::Sort::Type
                    } else {
                        nir::Sort::ComptimeValue
                    },
                )
            })
            .collect();
        construction::spend(&mut budget.steps, 1)?;
        Ok(Arc::new(Self {
            index,
            declaration_parameter_sorts,
            callable_functions,
            runtime_callables,
            normal_heads,
        }))
    }
}

impl std::fmt::Display for DeclarationSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-snapshot (source {}) (metadata",
            self.0.package
        )?;
        match self.0.metadata.get() {
            Some(metadata) => write!(f, " {}", metadata.index)?,
            None => f.write_str(" pending")?,
        }
        f.write_str("))")
    }
}
