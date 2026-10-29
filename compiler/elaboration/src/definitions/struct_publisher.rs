//! Checked host service retaining HIR and the latest completed NIR publication.
use super::*;
use comptime_interpretor::{Error, ErrorKind, PreparedStructBuild, StructBuildRequest};
use hir::{HirArena, Ident, Symbol};
use rustc_span::DUMMY_SP;
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

struct Published<'hir> {
    hir: hir::Package<'hir>,
    nir: Option<nir::program::Package>,
    revision: u64,
}

/// One committed package world. Candidates never borrow mutable inference state.
/// A host uses `prepare` as its VM callback, then consumes `finish` after all
/// invocations to recover the latest checked definitions without re-elaboration.
pub struct StructPublisher<'hir> {
    arena: &'hir HirArena,
    published: RefCell<Published<'hir>>,
    identities: BTreeMap<u32, u64>,
    resolution_package: u32,
    protocols: Protocols,
    target: Option<nir::target::TargetData>,
    structural_fuel: Cell<usize>,
    diagnostic: RefCell<Option<String>>,
}
impl<'hir> StructPublisher<'hir> {
    pub fn new(
        arena: &'hir HirArena,
        package: hir::Package<'hir>,
        resolution_package: u32,
        identities: BTreeMap<u32, u64>,
        protocols: Protocols,
        target: Option<nir::target::TargetData>,
        structural_fuel: usize,
    ) -> Result<Self, String> {
        if !identities.contains_key(&resolution_package) {
            return Err("builder destination requires an explicit package identity".into());
        }
        Ok(Self {
            arena,
            published: RefCell::new(Published {
                hir: package,
                nir: None,
                revision: 0,
            }),
            identities,
            resolution_package,
            protocols,
            target,
            structural_fuel: Cell::new(structural_fuel),
            diagnostic: RefCell::new(None),
        })
    }
    pub fn remaining_structural_fuel(&self) -> usize {
        self.structural_fuel.get()
    }
    pub fn last_diagnostic(&self) -> Option<String> {
        self.diagnostic.borrow().clone()
    }
    pub fn published_definition_count(&self) -> usize {
        self.published.borrow().hir.num_defs()
    }
    pub fn finish(self) -> (hir::Package<'hir>, Option<nir::program::Package>) {
        let published = self.published.into_inner();
        (published.hir, published.nir)
    }
    fn failure(&self, request: &StructBuildRequest, message: impl ToString) -> Error {
        *self.diagnostic.borrow_mut() = Some(message.to_string());
        Error {
            kind: ErrorKind::Type,
            source: Some(request.source),
        }
    }
    pub fn prepare<'s>(
        &'s self,
        request: &StructBuildRequest,
        invocation_steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.prepare_with_defaults(request, invocation_steps, |_, _, _, _| Ok(None))
    }
    /// Assemble defaults in the candidate's hygienic field scopes, then run
    /// ordinary elaboration before publication. The callback shares structural
    /// fuel and may use CMTT nodes; it must not capture live checker state.
    /// Defaults remain in the retained HIR returned by `finish`, for subsequent
    /// constructor elaboration. Installing the returned VM type descriptors
    /// alone does not install default expressions or suspend a running checker.
    pub fn prepare_with_defaults<'s>(
        &'s self,
        request: &StructBuildRequest,
        invocation_steps: &mut u64,
        mut make_default: impl FnMut(
            usize,
            hir::HirId,
            &mut hir::definitions::DefinitionNodes<'hir>,
            &mut usize,
        )
            -> Result<Option<&'hir hir::Expr<'hir>>, hir::definitions::Error>,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        *self.diagnostic.borrow_mut() = None;
        // Reserve before running any fallible construction; unwind cannot
        // replenish the structural allowance independently of invocation fuel.
        let mut fuel = self.structural_fuel.replace(0);
        let result = self.prepare_inner(request, invocation_steps, &mut fuel, &mut make_default);
        self.structural_fuel.set(fuel);
        result
    }
    fn prepare_inner<'s>(
        &'s self,
        request: &StructBuildRequest,
        invocation_steps: &mut u64,
        fuel: &mut usize,
        make_default: impl FnMut(
            usize,
            hir::HirId,
            &mut hir::definitions::DefinitionNodes<'hir>,
            &mut usize,
        )
            -> Result<Option<&'hir hir::Expr<'hir>>, hir::definitions::Error>,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        let published = self.published.borrow();
        let revision = published.revision;
        let next_revision = revision
            .checked_add(1)
            .ok_or_else(|| self.failure(request, "builder revision exhausted"))?;
        let mut candidate = hir::definitions::fork_package(&published.hir, fuel)
            .map_err(|error| self.failure(request, error))?;
        drop(published);
        let mut batch = DefinitionBatch::new(&mut candidate);
        let owner = batch
            .reserve()
            .map_err(|error| self.failure(request, error))?;
        let identity = batch
            .reserve_identity(self.resolution_package, fuel)
            .map_err(|error| self.failure(request, error))?;
        // Preflight the request copies before allocating the arena descriptions.
        for field in &request.fields {
            *fuel = fuel
                .checked_sub(field.name.len().saturating_add(1))
                .ok_or_else(|| self.failure(request, "builder structural budget exhausted"))?;
            nir::type_computations::charge_term(&field.ty, fuel)
                .map_err(|error| self.failure(request, format!("builder Type copy: {error:?}")))?;
        }
        *fuel = fuel
            .checked_sub(request.name.len())
            .ok_or_else(|| self.failure(request, "builder structural budget exhausted"))?;
        let fields: Vec<_> = request
            .fields
            .iter()
            .map(|field| hir::definitions::TypedField {
                name: Ident::new(Symbol::intern(&field.name), DUMMY_SP),
                ty: Arc::new(field.ty.clone()),
                span: DUMMY_SP,
            })
            .collect();
        let info = hir::definitions::assemble_struct_with_defaults(
            self.arena,
            owner,
            Ident::new(Symbol::intern(&request.name), DUMMY_SP),
            &fields,
            DUMMY_SP,
            fuel,
            make_default,
        )
        .map_err(|error| self.failure(request, error))?;
        batch
            .define(info, Some(identity), None)
            .map_err(|error| self.failure(request, error))?;
        let (_, checked) = publish_with_invocation_budget(
            &mut batch,
            fuel,
            invocation_steps,
            &self.identities,
            &self.protocols,
            self.target,
        )
        .map_err(|error| self.failure(request, error))?;
        let result = nir::DeclarationId {
            package: self.identities[&self.resolution_package],
            local: identity.index,
        };
        let definition = checked
            .structs
            .iter()
            .find(|item| item.declaration == result)
            .ok_or_else(|| self.failure(request, "checked builder result is missing"))?;
        // Copy the layout descriptor for VM admission. Default expressions stay
        // in candidate HIR for constructor elaboration; neither callbacks nor
        // active checker state are retained in the descriptor.
        for (_, ty) in &definition.fields {
            nir::type_computations::charge_term(ty, fuel).map_err(|error| {
                self.failure(request, format!("builder descriptor copy: {error:?}"))
            })?;
        }
        let names = definition
            .fields
            .iter()
            .try_fold(definition.name.len(), |sum, (name, _)| {
                sum.checked_add(name.len().saturating_add(1))
            })
            .ok_or_else(|| self.failure(request, "builder descriptor size overflow"))?;
        *fuel = fuel
            .checked_sub(names)
            .ok_or_else(|| self.failure(request, "builder descriptor budget exhausted"))?;
        let definitions = vec![definition.clone()].into();
        let source = request.source;
        Ok(PreparedStructBuild {
            definitions,
            result,
            commit: Box::new(move || {
                let mut published = self.published.borrow_mut();
                if published.revision != revision {
                    *self.diagnostic.borrow_mut() = Some(
                        "builder candidate is stale; prepare again against the committed package"
                            .into(),
                    );
                    return Err(Error {
                        kind: ErrorKind::InvalidCallPlan,
                        source: Some(source),
                    });
                }
                *published = Published {
                    hir: candidate,
                    nir: Some(checked),
                    revision: next_revision,
                };
                Ok(())
            }),
        })
    }
}
impl std::fmt::Display for StructPublisher<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let published = self.published.borrow();
        write!(
            f,
            "(struct-publisher (revision {}) (definitions {}) (structural-fuel {}))",
            published.revision,
            published.hir.num_defs(),
            self.structural_fuel.get()
        )
    }
}
